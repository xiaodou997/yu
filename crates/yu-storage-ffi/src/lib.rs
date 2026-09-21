#![allow(unsafe_code)]

//! Narrow C ABI for the macOS document-shell spike.
//!
//! `YuStorageSession` owns the only mutable `DocumentEditorSession`, which in
//! turn owns one `DocumentSession` and one `EditorDocument`. Native code may
//! request owned UTF-8 snapshots and revision-bound state, and can route
//! editor commands/IME composition through the same handle without creating a
//! second source. The AppKit host consumes only owned snapshots and explicit
//! result structs; its TextKit mirror is disposable and never canonical.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr;
#[cfg(target_os = "macos")]
mod macos_frame_worker;
#[cfg(target_os = "macos")]
use macos_frame_worker::LatestWorker;

#[cfg(target_os = "macos")]
use std::collections::{BTreeMap, HashSet};

#[cfg(target_os = "macos")]
use yu_assets::ImageKey;
#[cfg(target_os = "macos")]
use yu_core::Revision;
use yu_core::{LineIndex, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    ACCESSIBILITY_SEMANTIC_FLAG_EXPANDED, ACCESSIBILITY_SEMANTIC_FLAG_ORDERED,
    ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE, AccessibilitySemanticNode,
    AccessibilitySemanticSnapshot, AccessibilityTextError, AccessibilityTextSnapshot, Bias,
    CaretAffinity, CommandResult, EditorCommand, EditorDocumentError, OutlineTree, SearchResults,
    SelectionError, SourceSync, TableResizeCommit, TableResizeGesture, TableResizeGestureError,
    TableResizeTarget, VisualOffset, VisualText,
};
#[cfg(target_os = "macos")]
use yu_editor::{BlockView, CaretScrollRequest, ImageSpan, ViewportConfig, ViewportSpan};
// `begin_table_resize_for_test` 在非 macOS 上也跑（表格排版是中立的），所以这两个
// 类型在 test 构建里到处都要。**`cfg(macos)` 是错的**：clippy 看 lib target 说
// 它们 unused，看不见 lib test target。
#[cfg(any(target_os = "macos", test))]
use yu_editor::{LayoutConfig, LayoutPoint, TableResizeHit};
use yu_export::{ExportError, export_clipboard, import_html_fragment};
use yu_storage::{
    ClosePrompt, CloseRequest, CloseState, DiskState, DocumentEditorSession, ExternalFileState,
    RecoveryRecord, RecoveryStore, SaveOutcome, StorageError, Utf8Bom,
};
use yu_text::{EditError, TextSnapshot};

#[cfg(target_os = "macos")]
use yu_scene::{EditorDecorationPrimitiveRole, Point, Primitive, Rect};
#[cfg(target_os = "macos")]
use yu_workspace::{
    Appearance, FrameBuildKey, FrameBuildRequest, FrameGeometry, FrameKey, FrameTableResize,
    ViewportFrameBuildInput, ViewportFrameBuildOutput, ViewportFramePublication,
    ViewportRenderConfig,
};

#[cfg(target_os = "macos")]
use yu_assets::{
    EmbeddedFailureKind, EmbeddedRenderRequest, EmbeddedRequestResult, EmbeddedResourceCache,
    EmbeddedResourceKind, ImageCache, ImageFailureKind, ImageIntrinsicPublication,
    ImagePublication, ImageRequest, ImageRequestCandidate, ImageRequestPlan, ImageRequestPriority,
    ImageRequestResult,
};
#[cfg(target_os = "macos")]
use yu_embedded_client::{NativeRendererClient, RenderControl, RenderFailure};
#[cfg(target_os = "macos")]
use yu_font::FontRequest;
#[cfg(target_os = "macos")]
use yu_font::GlyphAtlasConfig;
#[cfg(target_os = "macos")]
use yu_font_macos::{CoreTextShaper, CoreTextViewportMetrics};
#[cfg(target_os = "macos")]
use yu_render::SurfaceConfig;
#[cfg(all(target_os = "macos", test))]
use yu_render_macos::MacosEmbeddedSvgRasterizer;
#[cfg(target_os = "macos")]
use yu_render_macos::{
    CoreTextViewportFrameBuilder, CoreTextViewportFrameError, MacosImageDecodeError,
    MacosImageDecodeWorker, MetalAtlas, MetalDevice, MetalFrameRenderer, MetalImageAtlas,
    MetalSurface, MetalUploader, MetalViewAttachmentOwned, MetalViewportHostSession,
};

pub const YU_STORAGE_OK: i32 = 0;
pub const YU_STORAGE_NULL_POINTER: i32 = 1;
pub const YU_STORAGE_INVALID_UTF8: i32 = 2;
pub const YU_STORAGE_IO_ERROR: i32 = 3;
pub const YU_STORAGE_EXTERNAL_CHANGE: i32 = 4;
pub const YU_STORAGE_UNSAVED_CHANGES: i32 = 5;
pub const YU_STORAGE_INVALID_PATH: i32 = 6;
pub const YU_STORAGE_EDITOR_ERROR: i32 = 7;
pub const YU_STORAGE_BUFFER_TOO_SMALL: i32 = 8;
pub const YU_STORAGE_INVALID_STATE: i32 = 9;
pub const YU_STORAGE_KEY_UNHANDLED: i32 = 10;
pub const YU_STORAGE_INVALID_COMMAND: i32 = 11;
pub const YU_STORAGE_INVALID_KEY: i32 = 12;
pub const YU_STORAGE_STALE_REVISION: i32 = 13;
pub const YU_STORAGE_INVALID_SELECTION: i32 = 14;
pub const YU_STORAGE_NO_OVERLAY: i32 = 15;
pub const YU_STORAGE_STALE_COMPOSITION: i32 = 16;
// 17 曾经是 YU_STORAGE_EXPORT_ERROR。S7 第六刀把 HTML 导出换成 comrak 之后，
// 导出只剩下 Revision 与 UTF-8 边界两种失败（`ExportError` 的两个变体），
// 没有任何一条路能产出它——一个没有生产者的状态码是 C ABI 上的一句谎话。
// **编号退休不复用**：别的平台可能还编译着旧头文件，17 换个含义比留一个空
// 洞危险得多。
pub const YU_STORAGE_HTML_IMPORT_REJECTED: i32 = 18;
pub const YU_STORAGE_SHAPER_UNAVAILABLE: i32 = 19;
pub const YU_STORAGE_INVALID_VIEWPORT_CONFIG: i32 = 20;
pub const YU_STORAGE_RENDER_HOST_UNAVAILABLE: i32 = 21;

pub const YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE: i32 = 22;
/// Temporary presentation backpressure; retry the latest geometry later.
pub const YU_STORAGE_RENDER_BUSY: i32 = 23;
pub const YU_STORAGE_INVALID_TABLE_PASTE: i32 = 24;
pub const YU_STORAGE_INVALID_RECOVERY: i32 = 25;

#[cfg(target_os = "macos")]
fn macos_surface_submit_error_status(error: yu_render_macos::MetalViewportHostError) -> i32 {
    match error {
        yu_render_macos::MetalViewportHostError::Render(
            yu_render_macos::MetalRenderError::DrawableUnavailable,
        ) => YU_STORAGE_RENDER_BUSY,
        _ => YU_STORAGE_RENDER_HOST_UNAVAILABLE,
    }
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn drawable_backpressure_is_retryable_but_render_failures_are_not() {
    use yu_render_macos::{MetalRenderError, MetalViewportHostError};
    assert_eq!(
        macos_surface_submit_error_status(MetalViewportHostError::Render(
            MetalRenderError::DrawableUnavailable,
        )),
        YU_STORAGE_RENDER_BUSY
    );
    assert_eq!(
        macos_surface_submit_error_status(MetalViewportHostError::Render(
            MetalRenderError::CommandBufferUnavailable,
        )),
        YU_STORAGE_RENDER_HOST_UNAVAILABLE
    );
    assert_eq!(
        macos_surface_submit_error_status(MetalViewportHostError::NoCurrentFrame {
            revision: Revision::INITIAL,
        }),
        YU_STORAGE_RENDER_HOST_UNAVAILABLE
    );
}

pub const YU_STORAGE_TABLE_ALIGNMENT_DEFAULT: u8 = 0;
pub const YU_STORAGE_TABLE_ALIGNMENT_LEFT: u8 = 1;
pub const YU_STORAGE_TABLE_ALIGNMENT_CENTER: u8 = 2;
pub const YU_STORAGE_TABLE_ALIGNMENT_RIGHT: u8 = 3;

pub const YU_STORAGE_TABLE_RESIZE_NONE: u8 = 0;
/// `yu_storage_session_copy_selection` 的输出格式。
/// 系统外观。平台每帧送一个字节进来，颜色由 `yu-workspace` 挑
/// （见 `yu_workspace::Appearance`）。
pub const YU_STORAGE_APPEARANCE_LIGHT: u8 = 0;
pub const YU_STORAGE_APPEARANCE_DARK: u8 = 1;
pub const YU_STORAGE_THEME_YU_LIGHT: u8 = 2;
pub const YU_STORAGE_THEME_YU_DARK: u8 = 3;

pub const YU_STORAGE_CLIPBOARD_TEXT: u8 = 0;
pub const YU_STORAGE_CLIPBOARD_HTML: u8 = 1;
pub const YU_STORAGE_CLIPBOARD_MARKDOWN: u8 = 2;
pub const YU_STORAGE_CLIPBOARD_FRAGMENTS: u8 = 3;
pub const YU_STORAGE_FRAGMENT_TEXT: u8 = 0;
pub const YU_STORAGE_FRAGMENT_MARKDOWN: u8 = 1;
pub const YU_STORAGE_FRAGMENT_HTML: u8 = 2;
pub const YU_STORAGE_FRAGMENT_HTML_TABLE: u8 = 3;

pub const YU_STORAGE_TABLE_RESIZE_COLUMN: u8 = 1;
pub const YU_STORAGE_TABLE_RESIZE_ROW: u8 = 2;

/// `yu_storage_session_table_resize_at_point` 的动作。
pub const YU_STORAGE_TABLE_RESIZE_PROBE: u8 = 0;
pub const YU_STORAGE_TABLE_RESIZE_BEGIN: u8 = 1;

/// `yu_storage_session_table_resize_action` 的动作。
pub const YU_STORAGE_TABLE_RESIZE_UPDATE: u8 = 0;
pub const YU_STORAGE_TABLE_RESIZE_FINISH: u8 = 1;
pub const YU_STORAGE_TABLE_RESIZE_CANCEL: u8 = 2;

pub const YU_STORAGE_SCENE_PRIMITIVE_BACKGROUND: u8 = 0;
pub const YU_STORAGE_SCENE_PRIMITIVE_TEXT_BOUNDS: u8 = 1;

pub const YU_STORAGE_COMMAND_DELETE_BACKWARD: u8 = 1;
pub const YU_STORAGE_COMMAND_DELETE_FORWARD: u8 = 2;
pub const YU_STORAGE_COMMAND_MOVE_LEFT: u8 = 3;
pub const YU_STORAGE_COMMAND_MOVE_RIGHT: u8 = 4;
pub const YU_STORAGE_COMMAND_INSERT_NEWLINE: u8 = 5;
pub const YU_STORAGE_COMMAND_INDENT_LIST: u8 = 6;
pub const YU_STORAGE_COMMAND_OUTDENT_LIST: u8 = 7;
pub const YU_STORAGE_COMMAND_UNDO: u8 = 8;
pub const YU_STORAGE_COMMAND_REDO: u8 = 9;
pub const YU_STORAGE_COMMAND_TOGGLE_TASK: u8 = 10;
pub const YU_STORAGE_COMMAND_MOVE_WORD_LEFT: u8 = 11;
pub const YU_STORAGE_COMMAND_MOVE_WORD_RIGHT: u8 = 12;
pub const YU_STORAGE_COMMAND_MOVE_UP: u8 = 13;
pub const YU_STORAGE_COMMAND_MOVE_DOWN: u8 = 14;
pub const YU_STORAGE_COMMAND_MOVE_UP_EXTEND: u8 = 15;
pub const YU_STORAGE_COMMAND_MOVE_DOWN_EXTEND: u8 = 16;
pub const YU_STORAGE_COMMAND_MOVE_LEFT_EXTEND: u8 = 17;
pub const YU_STORAGE_COMMAND_MOVE_RIGHT_EXTEND: u8 = 18;
pub const YU_STORAGE_COMMAND_MOVE_WORD_LEFT_EXTEND: u8 = 19;
pub const YU_STORAGE_COMMAND_MOVE_WORD_RIGHT_EXTEND: u8 = 20;
pub const YU_STORAGE_COMMAND_MOVE_DOCUMENT_START: u8 = 21;
pub const YU_STORAGE_COMMAND_MOVE_DOCUMENT_END: u8 = 22;
pub const YU_STORAGE_COMMAND_MOVE_DOCUMENT_START_EXTEND: u8 = 23;
pub const YU_STORAGE_COMMAND_MOVE_DOCUMENT_END_EXTEND: u8 = 24;
pub const YU_STORAGE_COMMAND_TABLE_NEXT: u8 = 25;
pub const YU_STORAGE_COMMAND_TABLE_PREVIOUS: u8 = 26;
pub const YU_STORAGE_COMMAND_DELETE_SELECTIONS: u8 = 27;
pub const YU_STORAGE_COMMAND_TABLE_INSERT_ROW_BEFORE: u8 = 28;
pub const YU_STORAGE_COMMAND_TABLE_INSERT_ROW_AFTER: u8 = 29;
pub const YU_STORAGE_COMMAND_TABLE_DELETE_ROW: u8 = 30;
pub const YU_STORAGE_COMMAND_TABLE_INSERT_COLUMN_BEFORE: u8 = 31;
pub const YU_STORAGE_COMMAND_TABLE_INSERT_COLUMN_AFTER: u8 = 32;
pub const YU_STORAGE_COMMAND_TABLE_DELETE_COLUMN: u8 = 33;
pub const YU_STORAGE_COMMAND_TABLE_ALIGN_LEFT: u8 = 34;
pub const YU_STORAGE_COMMAND_TABLE_ALIGN_CENTER: u8 = 35;
pub const YU_STORAGE_COMMAND_TABLE_ALIGN_RIGHT: u8 = 36;
pub const YU_STORAGE_COMMAND_TABLE_ALIGN_DEFAULT: u8 = 37;
pub const YU_STORAGE_COMMAND_DELETE_WORD_BACKWARD: u8 = 38;
pub const YU_STORAGE_COMMAND_DELETE_WORD_FORWARD: u8 = 39;

pub const YU_STORAGE_SOURCE_SYNC_NONE: u8 = 0;
pub const YU_STORAGE_SOURCE_SYNC_RANGE: u8 = 1;
pub const YU_STORAGE_SOURCE_SYNC_FULL: u8 = 2;
pub const YU_STORAGE_CARET_AFFINITY_UPSTREAM: u8 = 0;
pub const YU_STORAGE_CARET_AFFINITY_DOWNSTREAM: u8 = 1;
pub const YU_STORAGE_PROJECTION_BLOCK_BLANK_LINE: u8 = 0;
pub const YU_STORAGE_PROJECTION_BLOCK_REFERENCE_DEFINITION: u8 = 1;
pub const YU_STORAGE_PROJECTION_BLOCK_PARAGRAPH: u8 = 2;
pub const YU_STORAGE_PROJECTION_BLOCK_HEADING: u8 = 3;
pub const YU_STORAGE_PROJECTION_BLOCK_FENCED_CODE: u8 = 4;
pub const YU_STORAGE_PROJECTION_BLOCK_BLOCK_QUOTE: u8 = 5;
pub const YU_STORAGE_PROJECTION_BLOCK_LIST_ITEM: u8 = 6;
pub const YU_STORAGE_PROJECTION_BLOCK_TASK_LIST_ITEM: u8 = 7;
pub const YU_STORAGE_PROJECTION_INLINE: u8 = 0;
pub const YU_STORAGE_PROJECTION_FENCED_CODE: u8 = 1;
pub const YU_STORAGE_PROJECTION_REFERENCE_DEFINITION: u8 = 2;
pub const YU_STORAGE_PROJECTION_TASK_LIST: u8 = 3;
pub const YU_STORAGE_PROJECTION_HEADING: u8 = 4;
pub const YU_STORAGE_PROJECTION_BLOCK_QUOTE: u8 = 5;
pub const YU_STORAGE_PROJECTION_LIST: u8 = 6;
pub const YU_STORAGE_PROJECTION_TABLE: u8 = 7;

pub const YU_STORAGE_DISK_UNCHANGED: u8 = 0;
pub const YU_STORAGE_DISK_CHANGED: u8 = 1;
pub const YU_STORAGE_DISK_MISSING: u8 = 2;
pub const YU_STORAGE_BOM_ABSENT: u8 = 0;
pub const YU_STORAGE_BOM_PRESENT: u8 = 1;
pub const YU_STORAGE_CLOSE_OPEN: u8 = 0;
pub const YU_STORAGE_CLOSE_CLOSED: u8 = 1;
pub const YU_STORAGE_CLOSE_PROMPT_SAVE: u8 = 2;
pub const YU_STORAGE_CLOSE_PROMPT_EXTERNAL_CHANGED: u8 = 3;
pub const YU_STORAGE_CLOSE_PROMPT_EXTERNAL_MISSING: u8 = 4;
pub const YU_STORAGE_CLOSE_NOW: u8 = 0;
pub const YU_STORAGE_CLOSE_PROMPT: u8 = 1;
pub const YU_STORAGE_CLOSE_ALREADY_CLOSED: u8 = 2;

/// `yu_storage_session_close_resolve` 的收场方式。
pub const YU_STORAGE_CLOSE_RESOLVE_CANCEL: u8 = 0;
pub const YU_STORAGE_CLOSE_RESOLVE_SAVE: u8 = 1;
pub const YU_STORAGE_CLOSE_RESOLVE_DISCARD: u8 = 2;
pub const YU_STORAGE_CLOSE_RESOLVE_ABORT: u8 = 3;
pub const YU_STORAGE_EXTERNAL_CHANGED: u8 = YU_STORAGE_DISK_CHANGED;
pub const YU_STORAGE_EXTERNAL_MISSING: u8 = YU_STORAGE_DISK_MISSING;
pub const YU_STORAGE_ACCESSIBILITY_PARENT_NONE: u32 = u32::MAX;
/// 大纲里没有上一级标题的那几条（文档的根级标题）。
pub const YU_STORAGE_OUTLINE_PARENT_NONE: u32 = u32::MAX;
pub const YU_STORAGE_ACCESSIBILITY_NO_RANGE: u64 = u64::MAX;
pub const YU_STORAGE_ACCESSIBILITY_NO_ACTION_BLOCK: u64 = u64::MAX;
pub const YU_STORAGE_ACCESSIBILITY_FLAG_ORDERED: u8 = ACCESSIBILITY_SEMANTIC_FLAG_ORDERED;
pub const YU_STORAGE_ACCESSIBILITY_FLAG_EXPANDED: u8 = ACCESSIBILITY_SEMANTIC_FLAG_EXPANDED;
pub const YU_STORAGE_ACCESSIBILITY_FLAG_TASK_DONE: u8 = ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE;
pub const YU_STORAGE_ACCESSIBILITY_KIND_DOCUMENT: u8 = 1;
pub const YU_STORAGE_ACCESSIBILITY_KIND_HEADING: u8 = 2;
pub const YU_STORAGE_ACCESSIBILITY_KIND_PARAGRAPH: u8 = 3;
pub const YU_STORAGE_ACCESSIBILITY_KIND_CODE_BLOCK: u8 = 4;
pub const YU_STORAGE_ACCESSIBILITY_KIND_BLOCK_QUOTE: u8 = 5;
pub const YU_STORAGE_ACCESSIBILITY_KIND_LIST_ITEM: u8 = 6;
pub const YU_STORAGE_ACCESSIBILITY_KIND_TASK_LIST_ITEM: u8 = 7;
pub const YU_STORAGE_ACCESSIBILITY_KIND_EMPHASIS: u8 = 8;
pub const YU_STORAGE_ACCESSIBILITY_KIND_STRONG: u8 = 9;
pub const YU_STORAGE_ACCESSIBILITY_KIND_CODE_SPAN: u8 = 10;
pub const YU_STORAGE_ACCESSIBILITY_KIND_LINK: u8 = 11;
pub const YU_STORAGE_ACCESSIBILITY_KIND_IMAGE: u8 = 12;
pub const YU_STORAGE_ACCESSIBILITY_KIND_AUTOLINK: u8 = 13;
pub const YU_STORAGE_ACCESSIBILITY_KIND_REFERENCE_LINK: u8 = 14;
pub const YU_STORAGE_ACCESSIBILITY_KIND_REFERENCE_IMAGE: u8 = 15;
pub const YU_STORAGE_ACCESSIBILITY_KIND_DISCLOSURE: u8 = 16;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageState {
    pub revision: u64,
    pub saved_revision: u64,
    pub dirty: u8,
    pub disk_state: u8,
    pub bom: u8,
    pub close_state: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCloseRequest {
    pub result: u8,
    pub close_state: u8,
}

/// Revision-bound selection endpoints. `anchor_utf16` 与 `focus_utf16` 保留
/// 原生拖动的方向；有序区间由调用方取两者的 min/max 推导，不再单独占一个
/// ABI 入口。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageSelectionEndpoints {
    pub revision: u64,
    pub anchor_utf16: u64,
    pub focus_utf16: u64,
    pub affinity: u8,
}

/// Revision-bound source/visual caret mapping for the native projection
/// adapter. Both positions use UTF-16 units so AppKit can consume the result
/// without owning a second Markdown parser or coordinate model.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageProjectionCaret {
    pub revision: u64,
    pub source_utf16: u64,
    pub visual_utf16: u64,
    pub round_trip_source_utf16: u64,
    pub affinity: u8,
}

/// Revision-bound reverse selection mapping for a native visual mirror.
/// Non-collapsed visual boundaries use the outer source projection edges so
/// hidden Markdown delimiters remain part of the canonical source selection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageProjectionSourceSelection {
    pub revision: u64,
    pub visual_start_utf16: u64,
    pub visual_end_utf16: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub round_trip_visual_start_utf16: u64,
    pub round_trip_visual_end_utf16: u64,
    pub affinity: u8,
}

/// Revision-bound metrics-layout hit-test result. `x`/`y` are the snapped
/// projection-local caret point returned by `yu-layout`; they are not screen
/// coordinates and must be transformed by the native platform shell.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageProjectionHit {
    pub revision: u64,
    pub source_utf16: u64,
    pub visual_utf16: u64,
    pub round_trip_source_utf16: u64,
    /// Complete Markdown image source range when the hit landed on an image
    /// placement; both fields are `YU_STORAGE_IMAGE_DESTINATION_NONE` for a
    /// regular text hit.
    pub image_source_start_utf16: u64,
    pub image_source_end_utf16: u64,
    /// Exact formula/diagram object hit; independent of its caret edge.
    pub content_source_start_utf16: u64,
    pub content_source_end_utf16: u64,
    pub navigation_target_start_utf16: u64,
    pub navigation_target_end_utf16: u64,
    pub line: u64,
    pub x: f32,
    pub y: f32,
    pub affinity: u8,
}

/// Revision- and composition-generation-bound transient projection metadata.
/// Canonical source stays unchanged while the marked-text overlay is active;
/// visual selection ranges are measured in the projected UTF-16 stream.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCompositionProjection {
    pub revision: u64,
    pub generation: u64,
    pub replacement_start_utf16: u64,
    pub replacement_end_utf16: u64,
    pub preedit_selection_start_utf16: u64,
    pub preedit_selection_end_utf16: u64,
    pub visual_selection_start_utf16: u64,
    pub visual_selection_end_utf16: u64,
    pub projected_utf16_length: u64,
    pub projected_utf8_length: u64,
    /// The visual UTF-16 range occupied by the transient preedit. This is
    /// distinct from the canonical source replacement range because Markdown
    /// delimiters may be hidden by the projection.
    pub visual_replacement_start_utf16: u64,
    pub visual_replacement_end_utf16: u64,
}

/// Revision- and composition-generation-bound CoreText-shaped caret geometry
/// for the active marked-text projection. Coordinates are local to the
/// parser-owned block; visual UTF-16 ranges remain in the full projected
/// stream so a native host can pair geometry with its existing projection
/// metadata without reparsing Markdown.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageCompositionShapedCaret {
    pub revision: u64,
    pub generation: u64,
    pub source_utf16: u64,
    pub block_index: u64,
    pub visual_utf16: u64,
    pub round_trip_source_utf16: u64,
    pub line_index: u64,
    pub caret_x: f32,
    pub caret_y: f32,
    pub caret_width: f32,
    pub caret_height: f32,
    pub visual_selection_start_utf16: u64,
    pub visual_selection_end_utf16: u64,
    pub visual_replacement_start_utf16: u64,
    pub visual_replacement_end_utf16: u64,
    pub affinity: u8,
}

/// One task checkbox hit from the currently published macOS retained frame.
/// The marker range is the parser-owned `[ ]`/`[x]` source, while the bounds
/// remain in document-space scene coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageTaskCheckboxHit {
    pub revision: u64,
    pub block_index: u64,
    pub marker_start_utf16: u64,
    pub marker_end_utf16: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Revision-bound hit-test result for an internal visible table divider. The
/// `kind` field uses `YU_STORAGE_TABLE_RESIZE_COLUMN` or
/// `YU_STORAGE_TABLE_RESIZE_ROW`; `index` identifies the visible column/row
/// immediately before the divider and `position` is its local x/y coordinate.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageTableResizeHit {
    pub revision: u64,
    pub block_index: u64,
    pub kind: u8,
    pub index: u64,
    pub position: f32,
}

/// Revision-bound, document-space metadata for one visible table column
/// divider. This is a read-only accessibility/inspection contract: it does
/// not open a resize gesture or mutate source, selection, history or layout
/// state. `x`/`y` are document coordinates and `width`/`height` describe the
/// narrow divider hit region spanning the visible table.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageTableResizeAccessibilityDivider {
    pub revision: u64,
    pub block_index: u64,
    pub kind: u8,
    pub index: u64,
    pub column_count: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub table_source_start_utf16: u64,
    pub table_source_end_utf16: u64,
    /// VoiceOver 每次增减这条分隔线时的列宽步长，以表格自身的行高为基准。
    ///
    /// 这是策略而不是平台信息。平台此前为了算它必须单独查一次字体度量——为一个
    /// 辅助功能的微调常量留着一整个 FFI 入口（不变量 I3）。
    pub adjust_step: f32,
}

/// Revision-bound, source-neutral table geometry produced by a native resize
/// gesture. `final_position` and `delta` are updated for each pointer move;
/// releasing the pointer returns the same shape as the committed candidate.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageTableResizeCommit {
    pub revision: u64,
    pub block_index: u64,
    pub kind: u8,
    pub index: u64,
    pub initial_position: f32,
    pub final_position: f32,
    pub delta: f32,
}

/// Revision-bound source caret resolved through one block-local layout. The
/// point is local to the block and the visual offset is block-local UTF-16.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageBlockCaret {
    pub revision: u64,
    pub source_utf16: u64,
    pub block_index: u64,
    pub visual_utf16: u64,
    pub round_trip_source_utf16: u64,
    pub line_index: u64,
    pub caret_x: f32,
    pub caret_y: f32,
    pub caret_width: f32,
    pub caret_height: f32,
    pub affinity: u8,
    pub shaped: u8,
}

/// Revision-bound shaped caret geometry and the absolute document scroll
/// target required to reveal it in a native visual viewport.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageCaretScrollRequest {
    pub revision: u64,
    pub source_utf16: u64,
    pub block_index: u64,
    pub caret_x: f32,
    pub caret_y: f32,
    pub caret_width: f32,
    pub caret_height: f32,
    pub current_scroll_y: f32,
    pub target_scroll_y: f32,
    pub margin: f32,
    pub needs_scroll: u8,
}

pub const YU_STORAGE_RENDER_COMMAND_FILL_RECT: u8 = 0;
pub const YU_STORAGE_RENDER_COMMAND_GLYPH: u8 = 1;
pub const YU_STORAGE_RENDER_COMMAND_IMAGE: u8 = 2;
pub const YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG: u8 = 3;
pub const YU_STORAGE_RENDER_COMMAND_TASK_CHECKBOX: u8 = 4;
pub const YU_STORAGE_RENDER_PAGE_NONE: u32 = u32::MAX;
pub const YU_STORAGE_IMAGE_DESTINATION_NONE: u64 = u64::MAX;
pub const YU_STORAGE_IMAGE_INLINE: u8 = 0;
pub const YU_STORAGE_IMAGE_REFERENCE: u8 = 1;
pub const YU_STORAGE_IMAGE_RESOURCE_UNKNOWN: u8 = 0;
pub const YU_STORAGE_IMAGE_RESOURCE_PENDING: u8 = 1;
pub const YU_STORAGE_IMAGE_RESOURCE_READY: u8 = 2;
pub const YU_STORAGE_IMAGE_RESOURCE_FAILED: u8 = 3;
pub const YU_STORAGE_EMBEDDED_RESOURCE_UNKNOWN: u8 = 0;
pub const YU_STORAGE_EMBEDDED_RESOURCE_PENDING: u8 = 1;
pub const YU_STORAGE_EMBEDDED_RESOURCE_READY: u8 = 2;
pub const YU_STORAGE_EMBEDDED_RESOURCE_FAILED: u8 = 3;
pub const YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED: u8 = 4;
pub const YU_STORAGE_EMBEDDED_MATH: u8 = 0;
pub const YU_STORAGE_EMBEDDED_MERMAID: u8 = 1;

/// Revision-bound state published by the persistent macOS render host. This
/// is a scalar lifecycle contract: command/page bytes remain owned by Rust's
/// frame and atlas caches, while the native host can observe whether an edit,
/// scroll, resize or atlas miss produced a new frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageMacosRenderHostSnapshot {
    pub revision: u64,
    pub composition_generation: u64,
    pub frame_revision: u64,
    pub surface_generation: u64,
    pub frame_serial: u64,
    pub command_count: u64,
    pub upload_count: u64,
    pub damage_count: u64,
    pub atlas_page_count: u64,
    pub atlas_glyph_count: u64,
    pub atlas_bytes: u64,
    pub content_height: f32,
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub max_scroll_y: f32,
    pub viewport_width: f32,
    pub published: u8,
    /// Counts of semantic editor decoration layers retained by this exact
    /// frame. Native code uses them to disable its AppKit painter only after
    /// the submitted surface proves equivalent selection/caret coverage.
    pub selection_decoration_count: u64,
    pub caret_decoration_count: u64,
    /// 这一帧画了几处搜索命中底色（含「当前命中」那一处）。
    ///
    /// 它是**搜索高亮的判据**：真实窗口里唯一能证明「改了查询，画面真的跟着
    /// 变了」的量，而它来自场景，不来自搜索自己那条路。
    pub search_decoration_count: u64,
    /// 这一帧有几个字形被上了**代码高亮的颜色**（颜色与这一帧的正文色不同）。
    ///
    /// 它是代码高亮在真实窗口里的判据。自动化那几层压得住「角色算对了」与
    /// 「颜色查对了」，压不住的是**这一帧真的把它们画出去了**——第三刀与第四刀
    /// 各有一个缺陷是在自动化全绿之后才被真实窗口抓到的，两次都是颜色。
    ///
    /// 数的是**场景图元**，不是装饰、不是 `TextRole`：判据不来自被测的那条路。
    pub highlighted_glyph_count: u64,
    /// 当前可见范围内还有未落定的图片或内嵌资源，需要再提交一次去收割 worker
    /// 的结果。这个判断此前在平台侧，要三次纯查询往返才能得出。
    pub resource_refresh_pending: u8,
    pub resource_retry_pending: u8,
    pub layout_pending: u8,
}

/// 平台在一次帧提交中提供的几何。
///
/// 只有 AppKit 知道这些值（view bounds、clip view 滚动位置、backing scale）。
/// 除此之外的一切判断——Revision、composition generation、是否与上一帧等价、
/// 是否需要重算度量、是否需要重试资源——都由 Rust 完成，平台不再为了做决策
/// 而反复查询编辑状态。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageFrameGeometry {
    pub size: f32,
    pub max_width: f32,
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub surface_width: f64,
    pub surface_height: f64,
    pub scale: f64,
}

/// Scalar result from the opt-in real CAMetalLayer submit bridge. The view,
/// layer, renderer, atlas and command queue remain owned by the synchronous
/// Rust call; only lifecycle metadata crosses the ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageMacosRenderHostSurfaceSnapshot {
    pub revision: u64,
    pub composition_generation: u64,
    pub surface_generation: u64,
    pub frame_serial: u64,
    pub uploaded_pages: u64,
    pub uploaded_images: u64,
    pub command_count: u64,
    pub damage_count: u64,
    pub atlas_page_count: u64,
    pub image_resource_count: u64,
    pub image_request_count: u64,
    pub image_failure_count: u64,
    pub image_eviction_count: u64,
    pub image_atlas_eviction_count: u64,
    pub image_candidate_count: u64,
    pub image_duplicate_count: u64,
    pub image_visible_candidate_count: u64,
    pub image_overscan_candidate_count: u64,
    pub image_retry_count: u64,
    pub submitted: u8,
    /// Non-zero when the existing retained frame was presented at a new
    /// scroll origin without rebuilding Markdown layout or glyphs.
    pub presentation_reused: u8,
    pub selection_decoration_count: u64,
    pub caret_decoration_count: u64,
    /// 见 [`YuStorageMacosRenderHostSnapshot::search_decoration_count`]。
    pub search_decoration_count: u64,
    /// 见 [`YuStorageMacosRenderHostSnapshot::highlighted_glyph_count`]。
    pub highlighted_glyph_count: u64,
    /// 见 [`YuStorageMacosRenderHostSnapshot::resource_refresh_pending`]。
    pub resource_refresh_pending: u8,
    pub resource_retry_pending: u8,
    pub layout_pending: u8,
    /// 这一帧渲染出来的文档总高度。可滚动范围必须以它为准——平台没有第二套
    /// 布局可以推导这个值（不变量 I5）。
    pub content_height: f32,
}

/// Revision-bound source coordinates used by the native Accessibility adapter.
/// The snapshot is intentionally compact: text and line contents are queried
/// separately through the existing expected-revision source-range API.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageAccessibilitySnapshot {
    pub revision: u64,
    pub number_of_characters_utf16: u64,
    pub selection_start_utf16: u64,
    pub selection_end_utf16: u64,
    pub line_count: u64,
    pub selection_affinity: u8,
}

/// A logical line range bound to one Accessibility snapshot revision.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageAccessibilityRange {
    pub revision: u64,
    pub start_utf16: u64,
    pub end_utf16: u64,
}

/// Extended semantic node payload. The original
/// `YuStorageAccessibilityNode` ABI remains unchanged; native clients that
/// need URL/action metadata opt into the V2 fill function below.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageAccessibilityNodeV2 {
    pub revision: u64,
    pub index: u32,
    pub parent: u32,
    pub kind: u8,
    pub flags: u8,
    pub level: u8,
    pub reserved: u8,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub label_start_utf16: u64,
    pub label_end_utf16: u64,
    /// Source-backed destination range for link/image nodes. Both values are
    /// `YU_STORAGE_ACCESSIBILITY_NO_RANGE` when the node has no
    /// destination in the current Revision.
    pub destination_start_utf16: u64,
    pub destination_end_utf16: u64,
    /// Markdown block index accepted by `YU_STORAGE_COMMAND_TOGGLE_TASK`, or
    /// `YU_STORAGE_ACCESSIBILITY_NO_ACTION_BLOCK` for non-actionable nodes.
    pub action_block: u64,
}

/// 大纲里的一条标题。
///
/// 它与 [`YuStorageAccessibilityNodeV2`] 是**并列**的两份派生视图，不是一份
/// 套着另一份：语义树是扁平的（每个块都挂在 Document 下），大纲的全部内容
/// 恰恰是标题之间的层级。理由写在 `yu-editor/src/outline.rs` 的模块文档。
///
/// **导航不另开入口**：拿 `label_start_utf16` 调
/// `yu_storage_session_set_selection_endpoints`，再调
/// `yu_storage_session_shaped_caret_scroll_request`。滚动仍然走
/// viewport 那一条路，平台侧不自己算 y。
///
/// **树的形状与那两串文字都由 Rust 给**（`yu_editor::OutlineTree`）。这份表
/// 按文档顺序排，也就是前序；`child_count` 是直接孩子的条数。两者合起来足以
/// 无歧义地还原整棵树，壳里因此没有一次查表、也没有「父亲查不到怎么办」
/// 那一支。`parent` 留着，它与 `child_count` 互为对方的参照。
/// Font traits in UTF-16 coordinates of the displayed outline label.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageOutlineStyleRun {
    pub start_utf16: u64,
    pub end_utf16: u64,
    /// Bits: 1 = bold, 2 = italic, 4 = code. Other bits are zero.
    pub traits: u8,
    pub reserved: [u8; 7],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageOutlineItem {
    pub revision: u64,
    /// 这一条在数组里的下标，也是 `parent` 的取值域。
    pub index: u32,
    /// 上一级标题的 `index`，根级标题是 `YU_STORAGE_OUTLINE_PARENT_NONE`。
    pub parent: u32,
    /// 标题级别，1..=6。
    pub level: u8,
    pub reserved: [u8; 7],
    /// 这条标题所在的块索引。
    pub block: u64,
    /// 标题块的整段源码，含 `#` 前缀或 Setext 的下划线那一行。
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    /// 标题正文：大纲上显示的就是这一段，不含任何结构标记。
    pub label_start_utf16: u64,
    pub label_end_utf16: u64,
    /// **直接**孩子的条数。
    pub child_count: u64,
    /// 面板上显示的那一行文字，在同一次调用拷出的 UTF-8 缓冲里的位置。
    ///
    /// 它不是 `label_*_utf16` 那一段源码：行内标记已经减掉，Setext 的两行
    /// 也已经折成一行。
    pub display_utf8_offset: u64,
    pub display_utf8_length: u64,
    /// 跨刷新的身份，同一个缓冲里的位置。展开状态与选中行按它记。
    pub identity_utf8_offset: u64,
    pub identity_utf8_length: u64,
    /// Slice of the style-run buffer for this displayed label.
    pub style_offset: u64,
    pub style_count: u64,
}

/// 结果面板上的一行：一处命中，加上它显示成的那行字。
///
/// **原来这里还有 `block` 与 `block_*_utf16` 三个字段**，它们存在的唯一理由
/// 是让壳自己把「命中所在的那一行」裁进块里，好满足回报隐藏区间那个入口的
/// 前置条件。裁剪与减法都挪进 Rust 之后（`yu_editor::SearchResults`），
/// 那个入口没有了，这三个字段也就没有了消费者——留着就是三个没人用得上的
/// 数，与第六刀退休 `YU_STORAGE_EXPORT_ERROR = 17` 同一条规矩。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageSearchMatch {
    pub revision: u64,
    pub start_utf16: u64,
    pub end_utf16: u64,
    /// 面板上显示的那一行文字，在同一次调用拷出的 UTF-8 缓冲里的位置。
    pub display_utf8_offset: u64,
    pub display_utf8_length: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCommandResult {
    pub revision: u64,
    pub selection_start_utf16: u64,
    pub selection_end_utf16: u64,
    pub affinity: u8,
    pub changed: u8,
    pub source_sync: u8,
    pub source_start_utf16: u64,
    pub source_old_end_utf16: u64,
    pub source_new_start_utf16: u64,
    pub source_new_end_utf16: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCompositionState {
    pub revision: u64,
    pub generation: u64,
    pub replacement_start_utf16: u64,
    pub replacement_end_utf16: u64,
    pub selection_start_utf16: u64,
    pub selection_end_utf16: u64,
    pub preedit_utf8_length: u64,
    pub active: u8,
}

#[cfg(target_os = "macos")]
struct MacosImageResourceState {
    cache: ImageCache,
    worker: MacosImageDecodeWorker,
    publications: BTreeMap<u64, ImagePublication>,
    intrinsics: BTreeMap<u64, ImageIntrinsicPublication>,
    in_flight: HashSet<ImageKey>,
    visible_request_count: usize,
    candidate_count: usize,
    duplicate_count: usize,
    visible_candidate_count: usize,
    overscan_candidate_count: usize,
    retry_count: u64,
}

#[cfg(target_os = "macos")]
struct MacosEmbeddedResourceState {
    cache: EmbeddedResourceCache,
    jobs: std::sync::mpsc::Sender<EmbeddedRenderRequest>,
    results: std::sync::mpsc::Receiver<EmbeddedWorkerResult>,
    control: RenderControl,
    style: yu_assets::EmbeddedStyle,
    theme: yu_core::ThemeId,
    diagnostics: Vec<(Revision, TextRange, String)>,
    publications: Vec<yu_assets::EmbeddedRenderPublication>,
}

#[cfg(target_os = "macos")]
type EmbeddedWorkerResult = (
    EmbeddedRenderRequest,
    Result<yu_assets::EmbeddedRenderPayload, RenderFailure>,
);

#[cfg(target_os = "macos")]
impl MacosEmbeddedResourceState {
    fn new() -> Self {
        let (jobs, receiver) = std::sync::mpsc::channel::<EmbeddedRenderRequest>();
        let (sender, results) = std::sync::mpsc::channel();
        let control = RenderControl::default();
        let worker_control = control.clone();
        static DOCUMENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let document_id = DOCUMENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let helper = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|dir| dir.join("../Helpers/yu-document-renderer"))
            })
            .unwrap_or_default();
        #[cfg(test)]
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug/yu-document-renderer");
        let _ = std::thread::Builder::new()
            .name("yu-native-resources".into())
            .spawn(move || {
                let mut renderer = NativeRendererClient::new(helper, document_id);
                while let Ok(request) = receiver.recv() {
                    #[cfg(debug_assertions)]
                    if request.kind() == EmbeddedResourceKind::Math
                        && let Some(delay) = std::env::var("YU_TEST_MATH_DELAY_MS")
                            .ok()
                            .and_then(|value| value.parse::<u64>().ok())
                            .filter(|delay| *delay > 0 && *delay <= 30_000)
                    {
                        println!("yu-render-test resource=math delay_ms={delay}");
                        let until =
                            std::time::Instant::now() + std::time::Duration::from_millis(delay);
                        while std::time::Instant::now() < until
                            && worker_control.is_current(request.revision().get())
                        {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                    let result = renderer.render(&request, &worker_control);
                    if sender.send((request, result)).is_err() {
                        break;
                    }
                    yu_render_macos::notify_resource_completion();
                }
            });
        Self {
            cache: EmbeddedResourceCache::new(),
            jobs,
            results,
            control,
            style: yu_assets::EmbeddedStyle::default(),
            theme: yu_core::ThemeId::Github,
            diagnostics: Vec::new(),
            publications: Vec::new(),
        }
    }

    fn set_style(&mut self, style: yu_assets::EmbeddedStyle, theme: yu_core::ThemeId) {
        if self.style != style || self.theme != theme {
            // Drop closes the old worker. Its in-flight response cannot enter
            // the replacement presentation even when the source revision is unchanged.
            *self = Self::new();
            self.style = style;
            self.theme = theme;
        }
    }

    fn retain_publication(&mut self, publication: yu_assets::EmbeddedRenderPublication) {
        if self.publications.iter().any(|old| {
            old.source_range() == publication.source_range()
                && old.revision() == publication.revision()
                && old.generation() == publication.generation()
        }) {
            return;
        }
        self.publications
            .retain(|old| old.source_range() != publication.source_range());
        self.publications.push(publication);
        yu_render_macos::notify_resource_completion();
    }

    fn advance(&mut self, revision: Revision) -> Result<(), i32> {
        self.control.set_revision(revision.get());
        self.publications
            .retain(|publication| publication.revision() == revision);
        self.diagnostics
            .retain(|(version, _, _)| *version == revision);
        while let Ok((completed, result)) = self.results.try_recv() {
            if completed.revision() == revision {
                self.diagnostics
                    .retain(|(_, range, _)| *range != completed.source_range());
                let diagnostic = match &result {
                    Err(RenderFailure::InvalidSource(message) | RenderFailure::Worker(message)) => {
                        Some(message.clone())
                    }
                    _ => None,
                };
                if let Some(message) = diagnostic {
                    self.diagnostics
                        .push((revision, completed.source_range(), message));
                    self.publications
                        .retain(|old| old.source_range() != completed.source_range());
                }
            }
            let result = result.map_err(|error| match error {
                RenderFailure::InvalidSource(_) => yu_assets::EmbeddedRenderError::InvalidSource,
                RenderFailure::Cancelled | RenderFailure::Worker(_) => {
                    yu_assets::EmbeddedRenderError::Worker
                }
            });
            match self.cache.complete(completed, revision, result) {
                Ok(EmbeddedRequestResult::Ready(publication)) => {
                    self.retain_publication(publication)
                }
                Ok(_) | Err(yu_assets::EmbeddedCacheError::StaleRevision { .. }) => {}
                Err(_) => return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE),
            }
        }
        Ok(())
    }

    fn request_result(
        &mut self,
        request: EmbeddedRenderRequest,
        revision: Revision,
    ) -> Result<EmbeddedRequestResult, i32> {
        self.advance(revision)?;
        let _ = self.cache.request(request.clone());
        while let Some(job) = self.cache.pending() {
            if let Err(error) = self.jobs.send(job) {
                let _ = self.cache.complete(
                    error.0,
                    revision,
                    Err(yu_assets::EmbeddedRenderError::Worker),
                );
            }
        }
        let result = self.cache.request(request);
        if let EmbeddedRequestResult::Ready(publication) = &result {
            self.retain_publication(publication.clone());
        }
        Ok(result)
    }

    fn status_for(
        &mut self,
        request: EmbeddedRenderRequest,
        revision: Revision,
    ) -> Result<u8, i32> {
        Ok(macos_embedded_resource_status(
            self.request_result(request, revision)?,
        ))
    }

    /// 只有测试用到：断言默认 Math 渲染器确实产出可光栅化的 SVG。
    #[cfg(test)]
    fn publication_for(
        &mut self,
        request: EmbeddedRenderRequest,
        revision: Revision,
    ) -> Result<Option<yu_assets::EmbeddedRenderPublication>, i32> {
        match self.request_result(request, revision)? {
            EmbeddedRequestResult::Ready(publication) => Ok(Some(publication)),
            EmbeddedRequestResult::Pending | EmbeddedRequestResult::Failed(_) => Ok(None),
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacosEmbeddedResourceState {
    fn drop(&mut self) {
        self.control.close();
    }
}

#[cfg(target_os = "macos")]
fn macos_image_failure_kind(error: &MacosImageDecodeError) -> ImageFailureKind {
    match error {
        MacosImageDecodeError::UnsupportedPlatform => ImageFailureKind::Unsupported,
        MacosImageDecodeError::InvalidPath | MacosImageDecodeError::Location(_) => {
            ImageFailureKind::Io
        }
        MacosImageDecodeError::NativeDecodeFailed | MacosImageDecodeError::Decode(_) => {
            ImageFailureKind::Decode
        }
        MacosImageDecodeError::WorkerClosed => ImageFailureKind::Worker,
    }
}

#[cfg(target_os = "macos")]
impl MacosImageResourceState {
    fn new() -> Result<Self, i32> {
        Ok(Self {
            cache: ImageCache::new(),
            worker: MacosImageDecodeWorker::new()
                .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
            publications: BTreeMap::new(),
            intrinsics: BTreeMap::new(),
            in_flight: HashSet::new(),
            visible_request_count: 0,
            candidate_count: 0,
            duplicate_count: 0,
            visible_candidate_count: 0,
            overscan_candidate_count: 0,
            retry_count: 0,
        })
    }

    /// Geometry identity is document-local. Bitmap completions elsewhere in
    /// the process must not invalidate this document's caret/layout snapshot.
    fn geometry_version(&self, revision: Revision) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut sizes = BTreeMap::new();
        for (key, image) in &self.intrinsics {
            if image.revision() == revision {
                let dimensions = image.dimensions();
                sizes.insert(*key, (dimensions.width(), dimensions.height()));
            }
        }
        for (key, image) in &self.publications {
            if image.revision() == revision {
                sizes.insert(
                    *key,
                    (image.dimensions().width(), image.dimensions().height()),
                );
            }
        }
        if sizes.is_empty() {
            return 0;
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        sizes.hash(&mut hash);
        hash.finish()
    }

    fn sync(
        &mut self,
        plan: ImageRequestPlan,
        revision: yu_core::Revision,
        document_path: PathBuf,
        max_pixel_dimension: u32,
    ) -> Result<(), i32> {
        let stats = plan.stats();
        self.visible_request_count = stats.unique_count();
        self.candidate_count = stats.candidate_count();
        self.duplicate_count = stats.duplicate_count();
        self.visible_candidate_count = stats.visible_candidate_count();
        self.overscan_candidate_count = stats.overscan_candidate_count();
        self.cache.advance_retry_clock();
        while let Some(result) = self
            .worker
            .try_recv()
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?
        {
            let (request, result) = result.into_parts();
            self.in_flight.remove(request.key());
            match result {
                Ok(image) => {
                    let Ok(publication) = self.cache.publish_decoded(request, revision, image)
                    else {
                        continue;
                    };
                    self.publications
                        .insert(publication.key().fingerprint(), publication);
                }
                Err(error) => {
                    let _ = self.cache.record_failure(
                        request,
                        revision,
                        macos_image_failure_kind(&error),
                    );
                }
            }
        }

        self.publications.clear();
        self.intrinsics.clear();
        for request in plan.into_requests() {
            let request = request.with_max_pixel_dimension(max_pixel_dimension);
            let request_for_metadata = request.clone();
            if self.in_flight.contains(request.key()) {
                if let Some(intrinsic) = self.cache.intrinsic_publication(&request_for_metadata) {
                    self.intrinsics
                        .insert(intrinsic.key().fingerprint(), intrinsic);
                }
                continue;
            }
            let retry_candidate = self
                .cache
                .failure(request.key())
                .is_some_and(|failure| failure.revision() == request_for_metadata.revision());
            let result = self.cache.request(request);
            let retry_scheduled = retry_candidate && matches!(&result, ImageRequestResult::Pending);
            match result {
                ImageRequestResult::Ready(publication) => {
                    let intrinsic = publication.intrinsic_publication();
                    self.publications
                        .insert(publication.key().fingerprint(), publication);
                    self.intrinsics
                        .insert(intrinsic.key().fingerprint(), intrinsic);
                }
                ImageRequestResult::Pending | ImageRequestResult::Failed(_) => {}
            }
            if retry_scheduled {
                self.retry_count = self.retry_count.saturating_add(1);
            }
            if let Some(intrinsic) = self.cache.intrinsic_publication(&request_for_metadata) {
                self.intrinsics
                    .insert(intrinsic.key().fingerprint(), intrinsic);
            }
        }

        while let Some(request) = self.cache.pending() {
            if !self.in_flight.insert(request.key().clone()) {
                continue;
            }
            if self.worker.submit(request, document_path.clone()).is_err() {
                return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE);
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq)]
struct MacosFrameTicket {
    request: FrameBuildRequest,
    surface_generation: u64,
    binding_generation: u64,
    resource_generation: u64,
}

#[cfg(target_os = "macos")]
impl MacosFrameTicket {
    fn matches_input(
        &self,
        key: &FrameBuildKey,
        surface: u64,
        binding: u64,
        resources: u64,
    ) -> bool {
        self.request.key() == key
            && self.surface_generation == surface
            && self.binding_generation == binding
            && self.resource_generation == resources
    }

    fn accepts(&self, current: &Self) -> bool {
        self.request
            .accepts(current.request.key(), current.request.generation())
            && self.surface_generation == current.surface_generation
            && self.binding_generation == current.binding_generation
            && self.resource_generation == current.resource_generation
    }
}

#[cfg(target_os = "macos")]
struct MacosFrameBuildJob {
    input: ViewportFrameBuildInput,
    shaper: CoreTextShaper,
    config: ViewportRenderConfig,
    ticket: MacosFrameTicket,
    minimum_serial: u64,
}

#[cfg(target_os = "macos")]
struct MacosFrameBuildResult {
    ticket: MacosFrameTicket,
    output: Result<ViewportFrameBuildOutput, i32>,
}

#[cfg(target_os = "macos")]
type MacosFrameBuildWorker = LatestWorker<MacosFrameBuildJob, MacosFrameBuildResult>;

#[cfg(target_os = "macos")]
fn macos_new_frame_worker(initial_serial: u64) -> Option<MacosFrameBuildWorker> {
    let mut builder: Option<CoreTextViewportFrameBuilder> = None;
    let mut document: Option<yu_editor::LayoutContext> = None;
    let mut serial = initial_serial;
    LatestWorker::new(move |job: MacosFrameBuildJob, cancellation| {
        let MacosFrameBuildJob {
            shaper,
            input:
                ViewportFrameBuildInput {
                    request,
                    document: snapshot,
                    image_publications,
                    image_intrinsics,
                    embedded_publications,
                },
            config,
            ticket,
            minimum_serial,
        } = job;
        debug_assert_eq!(cancellation.generation(), ticket.request.generation());
        let start = std::time::Instant::now();
        let output = (|| {
            let rebuild = builder.as_ref().is_none_or(|builder| {
                builder.config().font_size() != config.font_size()
                    || builder.config().raster_scale() != config.raster_scale()
                    || builder.config().appearance() != config.appearance()
            });
            serial = serial.max(minimum_serial);
            if rebuild {
                if cancellation.is_cancelled() {
                    return Err(YU_STORAGE_RENDER_BUSY);
                }
                builder = Some(
                    CoreTextViewportFrameBuilder::with_shaper_and_initial_serial(
                        shaper,
                        config,
                        GlyphAtlasConfig::default(),
                        serial,
                    )
                    .map_err(|error| macos_render_host_error_status(&error))?,
                );
            }
            let builder = builder.as_mut().ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
            builder.set_embedded_publications(embedded_publications);
            // Cancellation can occur after raising the local serial floor but before
            // replacing the builder. Enforce the accepted floor on every job.
            builder.ensure_publication_serial(minimum_serial);
            builder
                .update_config(config)
                .map_err(|error| macos_render_host_error_status(&error))?;
            if cancellation.is_cancelled() {
                return Err(YU_STORAGE_RENDER_BUSY);
            }
            let reused = document
                .as_ref()
                .is_some_and(|document| snapshot.can_reuse_layout_context(document));
            let mut owned_document = if reused {
                let mut retained = document.take().ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
                snapshot.merge_into_layout_context(&mut retained);
                retained
            } else {
                snapshot.into_layout_context()
            };
            // Active input and the current viewport always precede distant work.
            let focus = owned_document.selection().focus();
            yu_workspace::prepare_layout_snapshot_with_resources_cancelable(
                &mut owned_document,
                yu_editor::LayoutQuery::Source(focus),
                builder.shaper(),
                &image_publications,
                &image_intrinsics,
                config.table_resize(),
                &mut || cancellation.is_cancelled(),
            )
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
            yu_workspace::prepare_layout_snapshot_with_resources_cancelable(
                &mut owned_document,
                config.viewport(),
                builder.shaper(),
                &image_publications,
                &image_intrinsics,
                config.table_resize(),
                &mut || cancellation.is_cancelled(),
            )
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
            // A changed input/width gets its visible publication first. Distant
            // measurement resumes on the next job with the same visual input.
            let measured = if reused { yu_workspace::measure_background_paragraphs_with_resources(
                &mut owned_document,
                builder.shaper(),
                &image_publications,
                &image_intrinsics,
                config.table_resize(),
                &mut || cancellation.is_cancelled(),
            )
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)? } else { 0 };
            if std::env::var_os("YU_RENDER_TIMING").is_some() {
                println!(
                    "yu-render-metric event=layout_batch measured={} remaining={} cached_paragraphs={} evicted_paragraphs={}",
                    measured,
                    owned_document.viewport_stats().entries()
                        - owned_document.viewport_stats().measured(),
                    owned_document.layout_cache_stats().entries(),
                    owned_document.layout_cache_stats().evicted(),
                );
            }
            let output = builder
                .publish_owned_document_cancelable(
                    request,
                    &mut owned_document,
                    image_publications,
                    image_intrinsics,
                    &mut || cancellation.is_cancelled(),
                )
                .map_err(|error| macos_render_host_error_status(&error));
            document = Some(owned_document);
            let output = output?;
            serial = output.publication.serial();
            Ok(output)
        })();
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!(
                "yu-render-metric event=preparation duration_ms={:.6}",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        MacosFrameBuildResult { ticket, output }
    })
    .ok()
}

#[cfg(target_os = "macos")]
struct MacosRenderHostState {
    builder: CoreTextViewportFrameBuilder,
    host: MetalViewportHostSession,
    // Accepted publication is authoritative for both worker and synchronous builds.
    last_publication: Option<ViewportFramePublication>,
    size: f32,
    surface: Option<MacosPersistentSurfaceState>,
    image_resources: MacosImageResourceState,
    /// 上一次成功提交的帧标识，用于跳过完全等价的重复提交。
    ///
    /// 这个判断此前在 Swift：平台每帧都要先查 Revision 和 composition
    /// generation 才能组装出比较用的键，一次提交因此产生七八次 FFI 往返。
    /// 状态在 Rust，决策就该在 Rust。
    last_frame_key: Option<FrameKey>,
    /// CPU publication identity, even while drawable submission is busy.
    prepared_build: Option<FrameBuildKey>,
    resource_refresh_pending: bool,
    resource_retry_pending: bool,
    layout_pending: bool,
    resource_completion_generation: u64,
    frame_worker: Option<MacosFrameBuildWorker>,
    frame_worker_request: Option<MacosFrameTicket>,
    frame_request_generation: u64,
    binding_generation: u64,
    visible_blocks: Vec<(usize, ImageRequestPriority)>,
    visibility_revision: Option<Revision>,
    metrics: CoreTextViewportMetrics,
}

/// 平台送来的外观字节。**未知值按浅色处理，不报错。**
///
/// 理由：外观不是一个操作，是一帧的属性。为一个认不得的枚举值拒绝整帧提交，
/// 表现是「系统换了个新外观之后 Yu 一片空白」——而按浅色画出来至少是可读的
/// （不变量 I5 的同一条精神：永不白屏）。
#[cfg(target_os = "macos")]
const fn appearance_from_raw(raw: u8) -> Appearance {
    match raw {
        YU_STORAGE_APPEARANCE_DARK => Appearance::Dark,
        YU_STORAGE_THEME_YU_LIGHT => Appearance::YuLight,
        YU_STORAGE_THEME_YU_DARK => Appearance::YuDark,
        _ => Appearance::Light,
    }
}

/// 把平台送过来的一帧几何翻成工作区那份定义。
///
/// 这一步留在 FFI，因为 `YuStorageFrameGeometry` 是 C 结构体；校验规则本身
/// 归 [`FrameGeometry`]，两端共用。
#[cfg(target_os = "macos")]
fn frame_geometry(request: &YuStorageFrameGeometry) -> Result<FrameGeometry, i32> {
    FrameGeometry::new(
        request.size,
        request.max_width,
        request.scroll_y,
        request.viewport_height,
        request.surface_width,
        request.surface_height,
        request.scale,
    )
    .ok_or(YU_STORAGE_EDITOR_ERROR)
}

/// 用当前会话状态与平台几何组装帧身份。
///
/// 提交路径与 `frame_is_current` 共用这一个函数。两边各写一份是这个判断最
/// 容易出错的地方：只要有一项不对称，就会出现「明明变了却判为等价」或
/// 「明明没变却每帧重画」。
#[cfg(target_os = "macos")]
fn frame_key(
    session: &YuStorageSession,
    appearance: Appearance,
    geometry: FrameGeometry,
) -> FrameKey {
    let revision = session.session.revision();
    // 与 `macos_render_host_frame` 使用同一条过滤规则：只有匹配当前
    // Revision 的列覆盖会进入渲染配置，其余不影响画面，也就不该影响身份。
    let table_resize = session
        .table_resize_override
        .filter(|commit| {
            commit.revision() == revision
                && matches!(commit.target(), TableResizeTarget::Column { .. })
        })
        .map(FrameTableResize::capture);
    FrameKey::new(
        revision.get(),
        session.session.composition_generation(),
        session.session.selections().as_slice().to_vec(),
        session.session.document().editor().search_generation(),
        table_resize,
        appearance,
        geometry,
    )
    .with_table_columns(session.session.selections().table_columns())
    .with_table_width_generation(session.session.document().editor().table_width_generation())
    .with_source_mode(session.session.document().editor().source_mode())
    .with_disclosure_fingerprint(
        session
            .session
            .document()
            .editor()
            .markdown()
            .html_disclosure_fingerprint(),
    )
    .with_focus_mode(session.focus_mode)
    .with_spelling_generation(session.session.document().editor().spelling_generation())
}

/// Toggle focus decoration without touching source, history or layout.
/// # Safety
/// A live session, used on its owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_focus_mode(
    session: *mut YuStorageSession,
    enabled: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if enabled > 1 {
        return YU_STORAGE_EDITOR_ERROR;
    }
    if session.focus_mode == (enabled != 0) {
        return YU_STORAGE_OK;
    }
    session.focus_mode = enabled != 0;
    #[cfg(target_os = "macos")]
    if let Some(state) = session.macos_render_host.as_mut() {
        if let Some(worker) = state.frame_worker.as_ref() {
            worker.invalidate();
        }
        state.frame_worker_request = None;
        state.last_frame_key = None;
        state.prepared_build = None;
    }
    YU_STORAGE_OK
}

/// Update local calendar context without editing source or history.
/// # Safety
/// A live session, called on the AppKit main thread when attached to a surface.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_set_reference_day(
    session: *mut YuStorageSession,
    day: i32,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if !(-719162..=2932896).contains(&day) {
        return YU_STORAGE_EDITOR_ERROR;
    }
    #[cfg(target_os = "macos")]
    {
        let resources = &mut session.macos_embedded_resources;
        if resources.style.reference_day() == Some(day) {
            return YU_STORAGE_OK;
        }
        resources.set_style(
            resources.style.with_reference_day(Some(day)),
            resources.theme,
        );
        if let Some(state) = session.macos_render_host.as_mut() {
            if let Some(worker) = state.frame_worker.as_ref() {
                worker.invalidate();
            }
            state.frame_worker_request = None;
            state.last_frame_key = None;
            state.prepared_build = None;
        }
        YU_STORAGE_OK
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = session;
        YU_STORAGE_EDITOR_ERROR
    }
}

/// Switch the canonical editor's projection without editing the source.
/// # Safety
/// A live session; call on the AppKit main thread when attached to a surface.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_source_mode(
    session: *mut YuStorageSession,
    enabled: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if enabled > 1 {
        return YU_STORAGE_EDITOR_ERROR;
    }
    if session.session.document().editor().source_mode() == (enabled != 0) {
        return YU_STORAGE_OK;
    }
    if session
        .session
        .document_mut()
        .editor_mut()
        .set_source_mode(enabled != 0)
        .is_err()
    {
        return YU_STORAGE_EDITOR_ERROR;
    }
    #[cfg(target_os = "macos")]
    if let Some(state) = session.macos_render_host.as_mut() {
        if let Some(worker) = state.frame_worker.as_ref() {
            worker.invalidate();
        }
        state.frame_worker_request = None;
        state.last_frame_key = None;
        state.prepared_build = None;
    }
    YU_STORAGE_OK
}

#[cfg(target_os = "macos")]
fn macos_frame_covers_viewport(
    frame: &yu_workspace::ViewportRenderFrame,
    scroll_y: f32,
    viewport_height: f32,
) -> bool {
    let blocks = frame.scene().input().blocks();
    let (Some(first), Some(last)) = (blocks.first(), blocks.last()) else {
        return false;
    };
    let coverage_top = first.y();
    let coverage_bottom = last.y() + last.height();
    let requested_bottom = scroll_y + viewport_height;
    scroll_y >= coverage_top - 0.5
        && requested_bottom <= coverage_bottom + 0.5
        && coverage_bottom >= coverage_top
}
#[cfg(target_os = "macos")]
struct MacosPersistentSurfaceState {
    surface: MetalSurface,
    attachment: Option<MetalViewAttachmentOwned>,
    renderer: MetalFrameRenderer,
    uploader: MetalUploader,
    atlas: MetalAtlas,
    image_atlas: MetalImageAtlas,
    view: std::ptr::NonNull<c_void>,
}

#[cfg(target_os = "macos")]
impl Drop for MacosPersistentSurfaceState {
    fn drop(&mut self) {
        // `MetalViewAttachmentOwned` must detach while the surface's native
        // layer is still retained. The explicit take keeps release ordering
        // deterministic; callers still explicitly detach on AppKit main thread
        // when the view/window is closing.
        self.attachment.take();
    }
}

#[repr(C)]
pub struct YuStorageSession {
    focus_mode: bool,
    session: DocumentEditorSession,
    table_resize_gesture: Option<TableResizeGesture>,
    table_resize_override: Option<TableResizeCommit>,
    #[cfg(target_os = "macos")]
    macos_render_host: Option<MacosRenderHostState>,
    #[cfg(target_os = "macos")]
    shader_library: Option<Vec<u8>>,
    #[cfg(target_os = "macos")]
    macos_embedded_resources: MacosEmbeddedResourceState,
}

/// Set the compiled library before attaching a native surface. No runtime compiler.
/// # Safety
/// The session must be live and the UTF-8 path readable for `length` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_shader_library(
    session: *mut YuStorageSession,
    path: *const u8,
    length: usize,
) -> i32 {
    if session.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let path = match read_utf8(path, length) {
        Ok(path) => path,
        Err(status) => return status,
    };
    #[cfg(target_os = "macos")]
    {
        let session = unsafe { &mut *session };
        if session
            .macos_render_host
            .as_ref()
            .is_some_and(|host| host.surface.is_some())
        {
            return YU_STORAGE_RENDER_HOST_UNAVAILABLE;
        }
        let library = match std::fs::read(path) {
            Ok(bytes) if bytes.starts_with(b"MTLB") => bytes,
            _ => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
        };
        session.shader_library = Some(library);
        YU_STORAGE_OK
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        YU_STORAGE_RENDER_HOST_UNAVAILABLE
    }
}

/// Release derived CPU/GPU caches on memory pressure. Source/history are retained.
/// # Safety
/// A live session, called on the AppKit main thread if a surface exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_trim_render_caches(
    session: *mut YuStorageSession,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    #[cfg(target_os = "macos")]
    {
        if let Some(state) = session.macos_render_host.as_mut() {
            let config = state.builder.config();
            let (shaper, _, _) = match core_text_layout(
                config.font_size(),
                config.scene_viewport().width(),
                config.appearance().theme_id(),
            ) {
                Ok(result) => result,
                Err(status) => return status,
            };
            let serial = state
                .last_publication
                .as_ref()
                .map_or(0, |publication| publication.serial());
            let builder = match CoreTextViewportFrameBuilder::with_shaper_and_initial_serial(
                shaper,
                config,
                GlyphAtlasConfig::default(),
                serial,
            ) {
                Ok(builder) => builder,
                Err(_) => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
            };
            if let Some(worker) = state.frame_worker.take() {
                worker.invalidate();
            }
            state.frame_worker = macos_new_frame_worker(serial);
            state.frame_worker_request = None;
            state.builder = builder;
            state.last_publication = None;
            state.last_frame_key = None;
            state.prepared_build = None;
            state.host = MetalViewportHostSession::new(
                session.session.revision(),
                state.host.surface_generation(),
            );
            state.image_resources.cache.clear_decoded();
            state.image_resources.publications.clear();
            if let Some(surface) = state.surface.as_mut() {
                surface.atlas = MetalAtlas::new();
                surface.image_atlas = MetalImageAtlas::new();
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = session;
    YU_STORAGE_OK
}

fn read_utf8<'a>(pointer: *const u8, length: usize) -> Result<&'a str, i32> {
    if length == 0 {
        return Ok("");
    }
    if pointer.is_null() {
        return Err(YU_STORAGE_NULL_POINTER);
    }
    // SAFETY: the native caller supplies a readable pointer/length pair that
    // remains valid for this synchronous call.
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    std::str::from_utf8(bytes).map_err(|_| YU_STORAGE_INVALID_UTF8)
}

fn write_bytes(bytes: &[u8], output: *mut u8, capacity: usize, written: *mut usize) -> i32 {
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` is a caller-owned output pointer checked above.
    unsafe { *written = bytes.len() };
    if bytes.is_empty() {
        return YU_STORAGE_OK;
    }
    // A null output with zero capacity is the ABI's length-query form. This
    // lets native callers size an owned snapshot without requiring a dummy
    // allocation or exposing Rust storage across the boundary.
    if output.is_null() {
        return if capacity == 0 {
            YU_STORAGE_OK
        } else {
            YU_STORAGE_NULL_POINTER
        };
    }
    if capacity < bytes.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    // SAFETY: capacity was checked against the source length.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len()) };
    YU_STORAGE_OK
}

fn status_from_error(error: StorageError) -> i32 {
    match error {
        StorageError::Io { .. } => YU_STORAGE_IO_ERROR,
        StorageError::InvalidUtf8 { .. } => YU_STORAGE_INVALID_UTF8,
        StorageError::InvalidPath(_) => YU_STORAGE_INVALID_PATH,
        StorageError::ExternalChange { .. } => YU_STORAGE_EXTERNAL_CHANGE,
        StorageError::UnsavedChanges { .. } => YU_STORAGE_UNSAVED_CHANGES,
        StorageError::CloseState(_) => YU_STORAGE_INVALID_STATE,
        StorageError::Editor(_) => YU_STORAGE_EDITOR_ERROR,
    }
}

fn status_from_export_error(error: ExportError) -> i32 {
    match error {
        ExportError::RevisionMismatch { .. } => YU_STORAGE_STALE_REVISION,
        ExportError::SourcePosition(_) => YU_STORAGE_INVALID_SELECTION,
    }
}

/// HTML import is intentionally a single, coarse native status. The native
/// adapter must treat every policy rejection as a signal to use plain text;
/// exposing parser internals here would make the C ABI depend on Markdown
/// implementation details.
fn status_from_html_import_error(_error: yu_export::HtmlImportError) -> i32 {
    YU_STORAGE_HTML_IMPORT_REJECTED
}

fn disk_state(session: &DocumentEditorSession) -> Result<u8, StorageError> {
    Ok(match session.disk_state()? {
        DiskState::Unchanged => YU_STORAGE_DISK_UNCHANGED,
        DiskState::Changed => YU_STORAGE_DISK_CHANGED,
        DiskState::Missing => YU_STORAGE_DISK_MISSING,
    })
}

fn close_state(session: CloseState) -> u8 {
    match session {
        CloseState::Open => YU_STORAGE_CLOSE_OPEN,
        CloseState::Closed => YU_STORAGE_CLOSE_CLOSED,
        CloseState::Prompting(ClosePrompt::SaveChanges) => YU_STORAGE_CLOSE_PROMPT_SAVE,
        CloseState::Prompting(ClosePrompt::ExternalChange {
            state: ExternalFileState::Changed,
        }) => YU_STORAGE_CLOSE_PROMPT_EXTERNAL_CHANGED,
        CloseState::Prompting(ClosePrompt::ExternalChange {
            state: ExternalFileState::Missing,
        }) => YU_STORAGE_CLOSE_PROMPT_EXTERNAL_MISSING,
    }
}

fn status_from_editor_error(error: EditorDocumentError) -> i32 {
    match error {
        EditorDocumentError::Edit(EditError::StaleRevision { .. })
        | EditorDocumentError::Selection(SelectionError::StaleRevision { .. }) => {
            YU_STORAGE_STALE_REVISION
        }
        EditorDocumentError::CompositionNotActive => YU_STORAGE_NO_OVERLAY,
        EditorDocumentError::InvalidTablePaste => YU_STORAGE_INVALID_TABLE_PASTE,
        EditorDocumentError::Selection(_) | EditorDocumentError::Composition(_) => {
            YU_STORAGE_INVALID_SELECTION
        }
        _ => YU_STORAGE_EDITOR_ERROR,
    }
}

fn status_from_accessibility_error(error: AccessibilityTextError) -> i32 {
    match error {
        AccessibilityTextError::StaleRevision { .. } => YU_STORAGE_STALE_REVISION,
        AccessibilityTextError::InvalidSourceRange(_)
        | AccessibilityTextError::InvalidUtf16Range(_)
        | AccessibilityTextError::Position(_)
        | AccessibilityTextError::OffsetOverflow
        | AccessibilityTextError::SemanticNodeOverflow
        | AccessibilityTextError::SemanticParse(_) => YU_STORAGE_INVALID_SELECTION,
    }
}

fn storage_status(error: StorageError) -> i32 {
    match error {
        StorageError::Editor(error) => status_from_editor_error(error),
        other => status_from_error(other),
    }
}

fn validate_composition(
    session: &DocumentEditorSession,
    expected_revision: u64,
    expected_generation: u64,
) -> Result<(), i32> {
    validate_revision(session, expected_revision)?;
    if session.composition_generation() != expected_generation {
        return Err(YU_STORAGE_STALE_COMPOSITION);
    }
    if session.composition().is_none() {
        return Err(YU_STORAGE_NO_OVERLAY);
    }
    Ok(())
}

fn caret_affinity_from_ffi(value: u8) -> Result<CaretAffinity, i32> {
    match value {
        YU_STORAGE_CARET_AFFINITY_UPSTREAM => Ok(CaretAffinity::Upstream),
        YU_STORAGE_CARET_AFFINITY_DOWNSTREAM => Ok(CaretAffinity::Downstream),
        _ => Err(YU_STORAGE_INVALID_SELECTION),
    }
}

fn selection_endpoints_from_ffi(
    session: &DocumentEditorSession,
    anchor_utf16: u64,
    focus_utf16: u64,
    affinity: u8,
) -> Result<yu_editor::EditorSelection, i32> {
    let affinity = caret_affinity_from_ffi(affinity)?;
    let snapshot = session.snapshot();
    let anchor = snapshot
        .byte_offset_for_utf16(Utf16Offset::new(anchor_utf16))
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let focus = snapshot
        .byte_offset_for_utf16(Utf16Offset::new(focus_utf16))
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    yu_editor::EditorSelection::range(&snapshot, anchor, focus, affinity)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)
}

fn source_range_from_ffi(
    session: &DocumentEditorSession,
    start_utf16: u64,
    end_utf16: u64,
) -> Result<TextRange, i32> {
    let range = Utf16Range::new(Utf16Offset::new(start_utf16), Utf16Offset::new(end_utf16))
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    let snapshot = session.snapshot();
    let start = snapshot
        .byte_offset_for_utf16(range.start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let end = snapshot
        .byte_offset_for_utf16(range.end())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    TextRange::new(start, end).ok_or(YU_STORAGE_INVALID_SELECTION)
}

fn selection_endpoints_output(
    session: &DocumentEditorSession,
    output: *mut YuStorageSelectionEndpoints,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let snapshot = session.snapshot();
    let selection = session.selection();
    let anchor_utf16 = match snapshot.utf16_offset(selection.anchor()) {
        Ok(offset) => offset.get(),
        Err(error) => {
            return status_from_editor_error(EditorDocumentError::Selection(error.into()));
        }
    };
    let focus_utf16 = match snapshot.utf16_offset(selection.focus()) {
        Ok(offset) => offset.get(),
        Err(error) => {
            return status_from_editor_error(EditorDocumentError::Selection(error.into()));
        }
    };
    // SAFETY: output is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageSelectionEndpoints {
            revision: session.revision().get(),
            anchor_utf16,
            focus_utf16,
            affinity: match selection.affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        };
    }
    YU_STORAGE_OK
}

fn accessibility_snapshot(
    session: &DocumentEditorSession,
) -> Result<AccessibilityTextSnapshot, i32> {
    let source = session.snapshot();
    let selection = session.selection();
    AccessibilityTextSnapshot::from_selection(source, selection)
        .map_err(status_from_accessibility_error)
}

fn accessibility_snapshot_output(
    session: &DocumentEditorSession,
    output: *mut YuStorageAccessibilitySnapshot,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let source = session.snapshot();
    let line_count = source.summary().line_count();
    let snapshot = match AccessibilityTextSnapshot::from_selection(source, session.selection()) {
        Ok(snapshot) => snapshot,
        Err(error) => return status_from_accessibility_error(error),
    };
    let selected = snapshot.selected_range().range();
    // SAFETY: `output` is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageAccessibilitySnapshot {
            revision: snapshot.revision().get(),
            number_of_characters_utf16: snapshot.number_of_characters().get(),
            selection_start_utf16: selected.start().get(),
            selection_end_utf16: selected.end().get(),
            line_count,
            selection_affinity: match session.selection().affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        };
    }
    YU_STORAGE_OK
}

fn accessibility_semantic_snapshot(
    session: &DocumentEditorSession,
) -> Result<AccessibilitySemanticSnapshot, i32> {
    AccessibilitySemanticSnapshot::from_document(session.document().editor())
        .map_err(status_from_accessibility_error)
}

fn accessibility_semantic_node_v2_output(
    node: AccessibilitySemanticNode,
) -> YuStorageAccessibilityNodeV2 {
    let source = node.source_range().range();
    let label = node.label_range().range();
    let (destination_start_utf16, destination_end_utf16) = node
        .destination_range()
        .map(|destination| {
            (
                destination.range().start().get(),
                destination.range().end().get(),
            )
        })
        .unwrap_or((
            YU_STORAGE_ACCESSIBILITY_NO_RANGE,
            YU_STORAGE_ACCESSIBILITY_NO_RANGE,
        ));
    YuStorageAccessibilityNodeV2 {
        revision: node.source_range().revision().get(),
        index: node.index(),
        parent: node
            .parent()
            .unwrap_or(YU_STORAGE_ACCESSIBILITY_PARENT_NONE),
        kind: node.kind().tag(),
        flags: node.flags(),
        level: node.level(),
        reserved: 0,
        source_start_utf16: source.start().get(),
        source_end_utf16: source.end().get(),
        label_start_utf16: label.start().get(),
        label_end_utf16: label.end().get(),
        destination_start_utf16,
        destination_end_utf16,
        action_block: node
            .action_block()
            .and_then(|block| u64::try_from(block).ok())
            .unwrap_or(YU_STORAGE_ACCESSIBILITY_NO_ACTION_BLOCK),
    }
}

fn write_accessibility_nodes_v2(
    nodes: &[AccessibilitySemanticNode],
    output: *mut YuStorageAccessibilityNodeV2,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` is a caller-owned output pointer checked above.
    unsafe { *written = nodes.len() };
    if nodes.is_empty() {
        return YU_STORAGE_OK;
    }
    if output.is_null() {
        return if capacity == 0 {
            YU_STORAGE_OK
        } else {
            YU_STORAGE_NULL_POINTER
        };
    }
    if capacity < nodes.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    let converted = nodes
        .iter()
        .copied()
        .map(accessibility_semantic_node_v2_output)
        .collect::<Vec<_>>();
    // SAFETY: capacity was checked against the number of converted nodes, and
    // the native caller supplied writable storage for that many values.
    unsafe {
        ptr::copy_nonoverlapping(converted.as_ptr(), output, converted.len());
    }
    YU_STORAGE_OK
}

/// 一次两遍协议的两个去处：定长条目一段，UTF-8 文本一段。
///
/// **它们必须出自同一次计算**——条目里的 `*_utf8_offset` 指进的正是这一次
/// 拷出来的那段文本。分成两个入口就有了两个可以对不上的答案（面板上那一行
/// 会显示成别人的字，不报错），所以两个缓冲区在一次调用里一起交出去。
struct PanelOutput<T> {
    items: *mut T,
    item_capacity: usize,
    item_count: *mut usize,
    text: *mut u8,
    text_capacity: usize,
    text_length: *mut usize,
}

impl<T> PanelOutput<T> {
    /// 只问长度的那一遍：两个指针都是 null、两个容量都是 0。
    const fn is_measuring(&self) -> bool {
        self.items.is_null() && self.text.is_null()
    }
}

/// 两遍协议：两个指针都传 null（容量都为 0）时只回报两个长度。
///
/// 与 `write_accessibility_nodes_v2` 同一个协议，多了一段文本。先把条目全部
/// 转完再拷——转到一半失败就拷了半份出去，那是「静默地做错事」；调用方在这
/// 一层已经拿到 `Vec` 与 `String`，所以拷贝本身不会失败。
fn write_panel_rows<T: Copy>(rows: &[T], text: &str, output: &PanelOutput<T>) -> i32 {
    if output.item_count.is_null() || output.text_length.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: both are caller-owned output pointers checked above.
    unsafe {
        *output.item_count = rows.len();
        *output.text_length = text.len();
    }
    if output.is_measuring() {
        return if output.item_capacity == 0 && output.text_capacity == 0 {
            YU_STORAGE_OK
        } else {
            YU_STORAGE_NULL_POINTER
        };
    }
    if output.items.is_null() || output.text.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if output.item_capacity < rows.len() || output.text_capacity < text.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    // SAFETY: both capacities were checked above, and the native caller
    // supplied writable storage for that many bytes and items.
    unsafe {
        if !rows.is_empty() {
            ptr::copy_nonoverlapping(rows.as_ptr(), output.items, rows.len());
        }
        if !text.is_empty() {
            ptr::copy_nonoverlapping(text.as_ptr(), output.text, text.len());
        }
    }
    YU_STORAGE_OK
}

/// 面板那一层的错误 → 状态码。
///
/// 都是「这一版拿不到」：调用方保留上一版比中止好，与 Revision 失配同一种
/// 处置。
const fn panel_status(_error: &yu_editor::PanelError) -> i32 {
    YU_STORAGE_EDITOR_ERROR
}

/// 把一段字符追加进文本缓冲，回报它落在哪儿。
fn push_display(text: &mut String, piece: &str) -> (u64, u64) {
    let offset = text.len() as u64;
    text.push_str(piece);
    (offset, piece.len() as u64)
}

/// 这一版大纲的全部行 + 它们那两串文字。
///
/// **树的形状、面板上那一行文字、跨刷新的身份都在 `yu-editor` 算**
/// （`OutlineTree`）。这里只做一件事：换算 UTF-16 偏移，把字符串摊进一段
/// 连续缓冲。
fn outline_rows_output(
    session: &mut DocumentEditorSession,
) -> Result<
    (
        Vec<YuStorageOutlineItem>,
        String,
        Vec<YuStorageOutlineStyleRun>,
    ),
    i32,
> {
    let snapshot = session.snapshot();
    let tree = OutlineTree::build(session.document_mut().editor_mut())
        .map_err(|error| panel_status(&error))?;
    let revision = tree.revision().get();
    let mut items = Vec::with_capacity(tree.rows().len());
    let mut text = String::new();
    let mut runs = Vec::new();
    for row in tree.rows() {
        let item = row.item();
        let (source_start_utf16, source_end_utf16) =
            source_utf16_range(&snapshot, item.source_range())?;
        let (label_start_utf16, label_end_utf16) =
            source_utf16_range(&snapshot, item.label_range())?;
        let (display_utf8_offset, display_utf8_length) = push_display(&mut text, row.label());
        let (identity_utf8_offset, identity_utf8_length) = push_display(&mut text, row.identity());
        let style_offset = runs.len() as u64;
        let mut byte_offset = 0;
        let mut utf16_offset = 0;
        for run in row.label_runs() {
            utf16_offset += row.label()[byte_offset..run.range.start]
                .encode_utf16()
                .count() as u64;
            let start_utf16 = utf16_offset;
            utf16_offset += row.label()[run.range.clone()].encode_utf16().count() as u64;
            byte_offset = run.range.end;
            runs.push(YuStorageOutlineStyleRun {
                start_utf16,
                end_utf16: utf16_offset,
                traits: u8::from(run.style.is_strong())
                    | (u8::from(run.style.is_emphasis()) << 1)
                    | (u8::from(run.style.is_code()) << 2),
                reserved: [0; 7],
            });
        }
        items.push(YuStorageOutlineItem {
            revision,
            index: u32::try_from(item.index()).map_err(|_| YU_STORAGE_BUFFER_TOO_SMALL)?,
            parent: item
                .parent()
                .map(|parent| u32::try_from(parent).map_err(|_| YU_STORAGE_BUFFER_TOO_SMALL))
                .transpose()?
                .unwrap_or(YU_STORAGE_OUTLINE_PARENT_NONE),
            level: item.level(),
            reserved: [0; 7],
            block: u64::try_from(item.block()).map_err(|_| YU_STORAGE_BUFFER_TOO_SMALL)?,
            source_start_utf16,
            source_end_utf16,
            label_start_utf16,
            label_end_utf16,
            child_count: u64::try_from(row.child_count())
                .map_err(|_| YU_STORAGE_BUFFER_TOO_SMALL)?,
            display_utf8_offset,
            display_utf8_length,
            identity_utf8_offset,
            identity_utf8_length,
            style_offset,
            style_count: runs.len() as u64 - style_offset,
        });
    }
    Ok((items, text, runs))
}

/// 当前查询这一版的全部结果行 + 它们那串文字。
///
/// 上下文裁剪（命中所在的那一行 ∩ 它所在的块）与减法都在 `yu-editor`
/// （`SearchResults`）。这里只换算 UTF-16 偏移。
fn search_rows_output(
    session: &mut DocumentEditorSession,
    revision: u64,
) -> Result<(Vec<YuStorageSearchMatch>, String), i32> {
    let snapshot = session.snapshot();
    let results = SearchResults::build(session.document_mut().editor_mut())
        .map_err(|error| panel_status(&error))?;
    let mut items = Vec::with_capacity(results.rows().len());
    let mut text = String::new();
    for row in results.rows() {
        let (start_utf16, end_utf16) = source_utf16_range(&snapshot, row.hit())?;
        let (display_utf8_offset, display_utf8_length) = push_display(&mut text, row.label());
        items.push(YuStorageSearchMatch {
            revision,
            start_utf16,
            end_utf16,
            display_utf8_offset,
            display_utf8_length,
        });
    }
    Ok((items, text))
}

fn accessibility_line_range_output(
    session: &DocumentEditorSession,
    expected_revision: u64,
    line: u64,
    output: *mut YuStorageAccessibilityRange,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let snapshot = match accessibility_snapshot(session) {
        Ok(snapshot) => snapshot,
        Err(status) => return status,
    };
    let range = match snapshot.range_for_line(LineIndex::new(line)) {
        Ok(range) => range.range(),
        Err(error) => return status_from_accessibility_error(error),
    };
    // SAFETY: `output` is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageAccessibilityRange {
            revision: snapshot.revision().get(),
            start_utf16: range.start().get(),
            end_utf16: range.end().get(),
        };
    }
    YU_STORAGE_OK
}

fn accessibility_line_for_position_output(
    session: &DocumentEditorSession,
    expected_revision: u64,
    offset_utf16: u64,
    output: *mut u64,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(session, expected_revision) {
        return status;
    }
    let snapshot = match accessibility_snapshot(session) {
        Ok(snapshot) => snapshot,
        Err(status) => return status,
    };
    let position = match snapshot.bind_position(Utf16Offset::new(offset_utf16)) {
        Ok(position) => position,
        Err(error) => return status_from_accessibility_error(error),
    };
    let line = match snapshot.line_for_position(position) {
        Ok(line) => line,
        Err(error) => return status_from_accessibility_error(error),
    };
    // SAFETY: `output` is checked above and belongs to the caller.
    unsafe { *output = line.get() };
    YU_STORAGE_OK
}

fn command_from_ffi(command: u8, block: u64) -> Result<EditorCommand, i32> {
    match command {
        YU_STORAGE_COMMAND_DELETE_BACKWARD => Ok(EditorCommand::DeleteBackward),
        YU_STORAGE_COMMAND_DELETE_FORWARD => Ok(EditorCommand::DeleteForward),
        YU_STORAGE_COMMAND_DELETE_WORD_BACKWARD => Ok(EditorCommand::DeleteWordBackward),
        YU_STORAGE_COMMAND_DELETE_WORD_FORWARD => Ok(EditorCommand::DeleteWordForward),
        YU_STORAGE_COMMAND_DELETE_SELECTIONS => Ok(EditorCommand::DeleteSelections),
        YU_STORAGE_COMMAND_MOVE_LEFT => Ok(EditorCommand::MoveLeft),
        YU_STORAGE_COMMAND_MOVE_RIGHT => Ok(EditorCommand::MoveRight),
        YU_STORAGE_COMMAND_INSERT_NEWLINE => Ok(EditorCommand::insert_newline()),
        YU_STORAGE_COMMAND_INDENT_LIST => Ok(EditorCommand::indent_list()),
        YU_STORAGE_COMMAND_OUTDENT_LIST => Ok(EditorCommand::outdent_list()),
        YU_STORAGE_COMMAND_UNDO => Ok(EditorCommand::undo()),
        YU_STORAGE_COMMAND_REDO => Ok(EditorCommand::redo()),
        YU_STORAGE_COMMAND_TOGGLE_TASK => usize::try_from(block)
            .map(EditorCommand::toggle_task)
            .map_err(|_| YU_STORAGE_INVALID_SELECTION),
        YU_STORAGE_COMMAND_MOVE_WORD_LEFT => Ok(EditorCommand::move_word_left()),
        YU_STORAGE_COMMAND_MOVE_WORD_RIGHT => Ok(EditorCommand::move_word_right()),
        YU_STORAGE_COMMAND_MOVE_UP => Ok(EditorCommand::move_up()),
        YU_STORAGE_COMMAND_MOVE_DOWN => Ok(EditorCommand::move_down()),
        YU_STORAGE_COMMAND_MOVE_UP_EXTEND => Ok(EditorCommand::move_up_extend()),
        YU_STORAGE_COMMAND_MOVE_DOWN_EXTEND => Ok(EditorCommand::move_down_extend()),
        YU_STORAGE_COMMAND_MOVE_LEFT_EXTEND => Ok(EditorCommand::ExtendHorizontal {
            forward: false,
            word: false,
        }),
        YU_STORAGE_COMMAND_MOVE_RIGHT_EXTEND => Ok(EditorCommand::ExtendHorizontal {
            forward: true,
            word: false,
        }),
        YU_STORAGE_COMMAND_MOVE_WORD_LEFT_EXTEND => Ok(EditorCommand::ExtendHorizontal {
            forward: false,
            word: true,
        }),
        YU_STORAGE_COMMAND_MOVE_WORD_RIGHT_EXTEND => Ok(EditorCommand::ExtendHorizontal {
            forward: true,
            word: true,
        }),
        YU_STORAGE_COMMAND_MOVE_DOCUMENT_START => Ok(EditorCommand::MoveDocumentBoundary {
            end: false,
            extend: false,
        }),
        YU_STORAGE_COMMAND_MOVE_DOCUMENT_END => Ok(EditorCommand::MoveDocumentBoundary {
            end: true,
            extend: false,
        }),
        YU_STORAGE_COMMAND_MOVE_DOCUMENT_START_EXTEND => Ok(EditorCommand::MoveDocumentBoundary {
            end: false,
            extend: true,
        }),
        YU_STORAGE_COMMAND_MOVE_DOCUMENT_END_EXTEND => Ok(EditorCommand::MoveDocumentBoundary {
            end: true,
            extend: true,
        }),
        YU_STORAGE_COMMAND_TABLE_INSERT_ROW_BEFORE => Ok(EditorCommand::EditTable(
            yu_editor::TableEdit::InsertRowBefore,
        )),
        YU_STORAGE_COMMAND_TABLE_INSERT_ROW_AFTER => Ok(EditorCommand::EditTable(
            yu_editor::TableEdit::InsertRowAfter,
        )),
        YU_STORAGE_COMMAND_TABLE_DELETE_ROW => {
            Ok(EditorCommand::EditTable(yu_editor::TableEdit::DeleteRow))
        }
        YU_STORAGE_COMMAND_TABLE_INSERT_COLUMN_BEFORE => Ok(EditorCommand::EditTable(
            yu_editor::TableEdit::InsertColumnBefore,
        )),
        YU_STORAGE_COMMAND_TABLE_INSERT_COLUMN_AFTER => Ok(EditorCommand::EditTable(
            yu_editor::TableEdit::InsertColumnAfter,
        )),
        YU_STORAGE_COMMAND_TABLE_DELETE_COLUMN => {
            Ok(EditorCommand::EditTable(yu_editor::TableEdit::DeleteColumn))
        }
        YU_STORAGE_COMMAND_TABLE_ALIGN_LEFT => {
            Ok(EditorCommand::EditTable(yu_editor::TableEdit::AlignLeft))
        }
        YU_STORAGE_COMMAND_TABLE_ALIGN_CENTER => {
            Ok(EditorCommand::EditTable(yu_editor::TableEdit::AlignCenter))
        }
        YU_STORAGE_COMMAND_TABLE_ALIGN_RIGHT => {
            Ok(EditorCommand::EditTable(yu_editor::TableEdit::AlignRight))
        }
        YU_STORAGE_COMMAND_TABLE_ALIGN_DEFAULT => {
            Ok(EditorCommand::EditTable(yu_editor::TableEdit::AlignDefault))
        }
        YU_STORAGE_COMMAND_TABLE_NEXT => Ok(EditorCommand::MoveTableCellNext),
        YU_STORAGE_COMMAND_TABLE_PREVIOUS => Ok(EditorCommand::MoveTableCellPrevious),

        _ => Err(YU_STORAGE_INVALID_COMMAND),
    }
}

fn command_result_output(
    session: &DocumentEditorSession,
    result: CommandResult,
    output: *mut YuStorageCommandResult,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let snapshot = session.snapshot();
    let selection = match result.selection().utf16_range(&snapshot) {
        Ok(selection) => selection,
        Err(error) => return status_from_editor_error(EditorDocumentError::Selection(error)),
    };
    let (source_sync, old_start, old_end, new_start, new_end) = match result.source_sync() {
        SourceSync::None => (YU_STORAGE_SOURCE_SYNC_NONE, 0, 0, 0, 0),
        SourceSync::Full if result.changed() => (YU_STORAGE_SOURCE_SYNC_FULL, 0, 0, 0, 0),
        SourceSync::Full => (YU_STORAGE_SOURCE_SYNC_NONE, 0, 0, 0, 0),
        SourceSync::Range(change) => (
            YU_STORAGE_SOURCE_SYNC_RANGE,
            change.old_range().start().get(),
            change.old_range().end().get(),
            change.new_range().start().get(),
            change.new_range().end().get(),
        ),
    };
    // SAFETY: output is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageCommandResult {
            revision: result.revision().get(),
            selection_start_utf16: selection.start().get(),
            selection_end_utf16: selection.end().get(),
            affinity: match result.selection().affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
            changed: u8::from(result.changed()),
            source_sync,
            source_start_utf16: old_start,
            source_old_end_utf16: old_end,
            source_new_start_utf16: new_start,
            source_new_end_utf16: new_end,
        };
    }
    YU_STORAGE_OK
}

fn write_snapshot_range(
    snapshot: &TextSnapshot,
    range: TextRange,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let start = match usize::try_from(range.start()) {
        Ok(start) => start,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let end = match usize::try_from(range.end()) {
        Ok(end) => end,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let required = end.saturating_sub(start);
    // SAFETY: `written` was checked above and belongs to the caller.
    unsafe { *written = required };
    if required == 0 {
        return YU_STORAGE_OK;
    }
    if output.is_null() {
        return if capacity == 0 {
            YU_STORAGE_OK
        } else {
            YU_STORAGE_NULL_POINTER
        };
    }
    if capacity < required {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    let mut copied = 0_usize;
    let mut chunks = match snapshot.chunk_cursor(range.start()) {
        Ok(chunks) => chunks,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    for chunk in &mut chunks {
        let chunk_start = match usize::try_from(chunk.start()) {
            Ok(start) => start,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let chunk_end = match chunk_start.checked_add(chunk.text().len()) {
            Some(end) => end,
            None => return YU_STORAGE_INVALID_SELECTION,
        };
        if chunk_start >= end {
            break;
        }
        let local_start = start.saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            let bytes = &chunk.text().as_bytes()[local_start..local_end];
            // SAFETY: capacity was checked against the requested byte range.
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), output.add(copied), bytes.len()) };
            copied += bytes.len();
        }
    }
    if copied == required {
        YU_STORAGE_OK
    } else {
        YU_STORAGE_INVALID_SELECTION
    }
}

/// 投影之后的视觉文本。
///
/// 它现在直接由 [`VisualText`] 持有——装饰应用一遍就得到它，不必再走一遍
/// run 列表。原来那段循环是 v1 的形状：`Projection` 只有 run，没有文本。
fn projected_utf8(text: &VisualText) -> String {
    text.text().to_owned()
}

fn visual_utf16_offset(projected: &str, visual: VisualOffset) -> Result<u64, i32> {
    let offset = usize::try_from(visual.get()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let prefix = projected
        .get(..offset)
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    u64::try_from(prefix.encode_utf16().count()).map_err(|_| YU_STORAGE_INVALID_SELECTION)
}

fn projection_bias_from_affinity(affinity: CaretAffinity) -> Bias {
    match affinity {
        CaretAffinity::Upstream => Bias::Before,
        CaretAffinity::Downstream => Bias::After,
    }
}

fn affinity_to_ffi(affinity: CaretAffinity) -> u8 {
    match affinity {
        CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
        CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
    }
}

/// 一段源码区间换成 UTF-16。跨 C ABI 的位置一律是 UTF-16（坐标规范）。
///
/// 原名 `table_source_utf16_range`，只有表格用。大纲是第二个消费者，名字里
/// 的 `table_` 跟着去掉。
fn source_utf16_range(snapshot: &TextSnapshot, source: TextRange) -> Result<(u64, u64), i32> {
    let start = snapshot
        .utf16_offset(source.start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let end = snapshot
        .utf16_offset(source.end())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    Ok((start, end))
}

// 存在面跟着它唯一的调用方 `begin_table_resize_session` 走。
#[cfg(any(target_os = "macos", test))]
fn table_resize_hit_metadata(
    revision: u64,
    block_index: u64,
    hit: TableResizeHit,
) -> Result<YuStorageTableResizeHit, i32> {
    let (kind, index) = match hit.target() {
        TableResizeTarget::Column { index } => (
            YU_STORAGE_TABLE_RESIZE_COLUMN,
            u64::try_from(index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        ),
        TableResizeTarget::Row { index } => (
            YU_STORAGE_TABLE_RESIZE_ROW,
            u64::try_from(index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        ),
    };
    Ok(YuStorageTableResizeHit {
        revision,
        block_index,
        kind,
        index,
        position: hit.position(),
    })
}

#[cfg(target_os = "macos")]
fn table_resize_accessibility_metadata(
    snapshot: &TextSnapshot,
    revision: u64,
    block_index: u64,
    block_y: f32,
    divider_width: f32,
    table: &yu_editor::TableLayout,
) -> Result<Vec<YuStorageTableResizeAccessibilityDivider>, i32> {
    let geometry = yu_workspace::ViewportTableGeometry::from_layout(
        usize::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        block_y,
        table,
    )
    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    table_resize_frame_geometry_metadata(snapshot, revision, divider_width, &geometry)
}

#[cfg(target_os = "macos")]
fn table_resize_frame_geometry_metadata(
    snapshot: &TextSnapshot,
    revision: u64,
    divider_width: f32,
    table: &yu_workspace::ViewportTableGeometry,
) -> Result<Vec<YuStorageTableResizeAccessibilityDivider>, i32> {
    let column_count =
        u64::try_from(table.column_widths().len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    if column_count < 2 {
        return Ok(Vec::new());
    }
    let (table_source_start_utf16, table_source_end_utf16) =
        source_utf16_range(snapshot, table.source())?;
    let bounds = table.bounds();
    let width = divider_width.max(1.0);
    let y = bounds.y();
    if !divider_width.is_finite()
        || divider_width <= 0.0
        || !y.is_finite()
        || !bounds.height().is_finite()
        || bounds.height() <= 0.0
    {
        return Err(YU_STORAGE_INVALID_SELECTION);
    }
    // 大约半个行高，并夹在 8–16 之间：一次调整要看得见，但不能一步跳过整列。
    // 行高不是常数（格内换行会撑高一行），取表头那一行——它是最稳定的那个，
    // 而这里要的只是一个手感常数，不是几何。
    let adjust_step = table.adjust_step();
    let block_index =
        u64::try_from(table.block_index()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let divider_count = table.column_widths().len().saturating_sub(1);
    let mut x = bounds.x();
    let mut dividers = Vec::with_capacity(divider_count);
    for (index, column_width) in table
        .column_widths()
        .iter()
        .copied()
        .take(divider_count)
        .enumerate()
    {
        x += column_width;
        if !x.is_finite() {
            return Err(YU_STORAGE_INVALID_SELECTION);
        }
        dividers.push(YuStorageTableResizeAccessibilityDivider {
            revision,
            block_index,
            kind: YU_STORAGE_TABLE_RESIZE_COLUMN,
            index: u64::try_from(index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
            column_count,
            x,
            y,
            width,
            height: bounds.height(),
            table_source_start_utf16,
            table_source_end_utf16,
            adjust_step,
        });
    }
    Ok(dividers)
}

/// Attached windows never rebuild layout for AX enumeration. An unavailable
/// or stale publication yields no descriptors until the next surface update.
/// Standalone/headless callers retain the existing synchronous query below.
#[cfg(target_os = "macos")]
fn macos_published_table_dividers(
    session: &YuStorageSession,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
) -> Option<Result<Vec<YuStorageTableResizeAccessibilityDivider>, i32>> {
    let state = session.macos_render_host.as_ref()?;
    state.surface.as_ref()?;
    if std::env::var_os("YU_RENDER_TIMING").is_some() {
        println!("yu-render-metric event=ax_frame_query");
    }
    let Some(frame) =
        macos_matching_submitted_frame(session, size, max_width, scroll_y, viewport_height)
    else {
        return Some(Ok(Vec::new()));
    };
    if std::env::var_os("YU_RENDER_TIMING").is_some() {
        println!(
            "yu-render-metric event=ax_frame_geometry tables={}",
            frame.scene().tables().len()
        );
    }
    if frame.scene().tables().is_empty() {
        return Some(Ok(Vec::new()));
    }
    let source = session.session.snapshot();
    let divider_width = (state.metrics.default_advance() * 0.25).max(1.0);
    let mut encoded = Vec::new();
    for table in frame.scene().tables() {
        match table_resize_frame_geometry_metadata(
            &source,
            frame.revision().get(),
            divider_width,
            table,
        ) {
            Ok(metadata) => encoded.extend(metadata),
            Err(status) => return Some(Err(status)),
        }
    }
    Some(Ok(encoded))
}

/// Validates the exact publication used by read-only window geometry queries.
#[cfg(target_os = "macos")]
fn macos_matching_submitted_frame(
    session: &YuStorageSession,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
) -> Option<std::sync::Arc<yu_workspace::ViewportRenderFrame>> {
    let state = session.macos_render_host.as_ref()?;
    let surface = state.surface.as_ref()?;
    if session.session.document().editor().composition().is_some() {
        return None;
    }
    let config = surface.surface.config();
    let geometry = FrameGeometry::new(
        size,
        max_width,
        scroll_y,
        viewport_height,
        config.logical_width(),
        config.logical_height(),
        config.scale(),
    )?;
    let requested = frame_key(session, state.builder.config().appearance(), geometry);
    if state.prepared_build.as_ref() != Some(requested.build())
        || state.host.surface_generation() != surface.surface.generation()
        || !state
            .host
            .last_submission()
            .is_some_and(|submission| Some(submission.frame_serial()) == state.host.frame_serial())
    {
        return None;
    }
    let frame = state.host.frame_handle()?;
    if frame.plan().viewport().y() != scroll_y
        && !macos_frame_covers_viewport(&frame, scroll_y, viewport_height)
    {
        return None;
    }
    Some(frame)
}

fn table_resize_commit_metadata(
    commit: TableResizeCommit,
) -> Result<YuStorageTableResizeCommit, i32> {
    let (kind, index) = match commit.target() {
        TableResizeTarget::Column { index } => (
            YU_STORAGE_TABLE_RESIZE_COLUMN,
            u64::try_from(index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        ),
        TableResizeTarget::Row { index } => (
            YU_STORAGE_TABLE_RESIZE_ROW,
            u64::try_from(index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        ),
    };
    Ok(YuStorageTableResizeCommit {
        revision: commit.revision().get(),
        block_index: u64::try_from(commit.block_index())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        kind,
        index,
        initial_position: commit.initial_position(),
        final_position: commit.final_position(),
        delta: commit.delta(),
    })
}

// 产品链路上只有 macOS 那半调它，但 `begin_table_resize_for_test` 在任何平台的
// test 构建里都调——表格排版本身是中立的。
#[cfg(any(target_os = "macos", test))]
fn begin_table_resize_session(
    session: &mut YuStorageSession,
    block_index: usize,
    hit: TableResizeHit,
    pointer_position: f32,
) -> Result<YuStorageTableResizeHit, i32> {
    if session.table_resize_gesture.is_some() {
        return Err(YU_STORAGE_INVALID_STATE);
    }
    let gesture = TableResizeGesture::begin(
        session.session.revision(),
        block_index,
        hit,
        pointer_position,
    )
    .map_err(table_resize_gesture_status)?;
    let block_index = u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let metadata = table_resize_hit_metadata(session.session.revision().get(), block_index, hit)?;
    session.table_resize_override = Some(gesture.preview());
    session.table_resize_gesture = Some(gesture);
    Ok(metadata)
}

#[cfg(target_os = "macos")]
fn block_caret_from_layout(
    session: &DocumentEditorSession,
    block_index: usize,
    source_utf16: u64,
    affinity: CaretAffinity,
    layout: &BlockView,
    _line_height: f32,
    shaped: u8,
) -> Result<YuStorageBlockCaret, i32> {
    if layout.revision() != session.revision() {
        return Err(YU_STORAGE_STALE_REVISION);
    }
    let snapshot = session.snapshot();
    let source = snapshot
        .byte_offset_for_utf16(Utf16Offset::new(source_utf16))
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let Some((source_range, _)) = session.block_metadata(block_index) else {
        return Err(YU_STORAGE_INVALID_SELECTION);
    };
    if !source_range.contains(source) && source != source_range.end() {
        return Err(YU_STORAGE_INVALID_SELECTION);
    }
    let bias = projection_bias_from_affinity(affinity);
    let caret = layout
        .caret_for_source(source, bias)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let projected = projected_utf8(layout.visual());
    let visual_utf16 = visual_utf16_offset(&projected, caret.visual())?;
    let round_trip = layout
        .visual()
        .visual_to_source(caret.visual(), bias)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let round_trip_source_utf16 = snapshot
        .utf16_offset(round_trip)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let point = caret.point();
    let line_height = layout.caret_line_height(caret);
    if !point.x().is_finite()
        || !point.y().is_finite()
        || !line_height.is_finite()
        || line_height <= 0.0
    {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    Ok(YuStorageBlockCaret {
        revision: session.revision().get(),
        source_utf16,
        block_index: u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        visual_utf16,
        round_trip_source_utf16,
        line_index: u64::try_from(caret.line()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        caret_x: point.x(),
        caret_y: point.y(),
        caret_width: 0.0,
        caret_height: line_height,
        affinity: affinity_to_ffi(affinity),
        shaped,
    })
}

/// viewport 配置与 CoreText 度量之间允许的偏差。
///
/// 度量来自浮点排版计算，逐次重算会有末位差异；容差必须大于该差异，否则每帧
/// 都会判为「配置不一致」而重建 ViewportLayout，把已缓存的 block 高度全部丢掉。
#[cfg(target_os = "macos")]
const MACOS_VIEWPORT_CONFIG_TOLERANCE: f32 = 0.05;

/// 让会话的 viewport 配置与这一次的 CoreText 度量对齐。
///
/// 这里此前是十份逐字相同的校验：Rust 自己用 CoreText 算出行高与默认步进，
/// 却要求平台先把同样的值原样送回来，不一致就整个调用失败。平台在这条链路上
/// 没有任何独有信息——它先调 `macos_font_metrics` 取值，再调
/// `set_viewport_config` 送回，每帧两次纯往返，换来的只是一个 Rust 本来就
/// 知道的数字。校验因此改为发布：由 Rust 自己保证配置正确（不变量 I3）。
///
/// 只在超出容差时才真正写入：`set_viewport_config` 会重建 `ViewportLayout`，
/// 连带丢掉已缓存的 block 高度，那正是 J2「按 block height index 定位可见范围」
/// 依赖的东西。
#[cfg(target_os = "macos")]
fn macos_publish_viewport_config(
    session: &mut YuStorageSession,
    max_width: f32,
    metrics: CoreTextViewportMetrics,
    theme: yu_core::ThemeId,
) -> Result<(), i32> {
    let resource_geometry = session.macos_render_host.as_ref().map_or(0, |state| {
        state
            .image_resources
            .geometry_version(session.session.revision())
    });
    session
        .session
        .document_mut()
        .editor_mut()
        .set_resource_geometry_version(resource_geometry);
    let published = session.session.viewport_config().layout();
    if published.theme() == theme
        && published.base_direction() == yu_core::BaseDirection::Ltr
        && (published.max_width() - max_width).abs() <= MACOS_VIEWPORT_CONFIG_TOLERANCE
        && (published.line_height() - metrics.line_height()).abs()
            <= MACOS_VIEWPORT_CONFIG_TOLERANCE
        && (published.default_advance() - metrics.default_advance()).abs()
            <= MACOS_VIEWPORT_CONFIG_TOLERANCE
    {
        return Ok(());
    }
    let layout = LayoutConfig::new(max_width, metrics.line_height())
        .with_theme(theme)
        .with_base_direction(yu_core::BaseDirection::Ltr)
        .with_default_advance(metrics.default_advance());
    // estimated_block_height 取一个行高、overscan 取 0，与平台此前送回来的
    // 值一致。这两项是策略而不是平台信息，因此现在由 Rust 决定。
    let config = ViewportConfig::new(layout, metrics.line_height(), 0.0);
    session
        .session
        .set_viewport_config(config)
        .map_err(|_| YU_STORAGE_INVALID_VIEWPORT_CONFIG)
}

#[cfg(target_os = "macos")]
fn macos_query_text_layout(
    session: &YuStorageSession,
    size: f32,
    max_width: f32,
    theme: yu_core::ThemeId,
) -> Result<(CoreTextShaper, CoreTextViewportMetrics, LayoutConfig), i32> {
    if !size.is_finite() || size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    if let Some(state) = session
        .macos_render_host
        .as_ref()
        .filter(|state| state.builder.config().appearance().theme_id() == theme)
    {
        let shaper = state
            .builder
            .shaper()
            .resized(size)
            .map_err(|_| YU_STORAGE_SHAPER_UNAVAILABLE)?;
        let metrics = if state.size == size {
            state.metrics
        } else {
            shaper
                .viewport_metrics("M中🙂e\u{301}")
                .map_err(|_| YU_STORAGE_SHAPER_UNAVAILABLE)?
        };
        let config = LayoutConfig::new(max_width, metrics.line_height())
            .with_theme(theme)
            .with_base_direction(yu_core::BaseDirection::Ltr)
            .with_default_advance(metrics.default_advance());
        return Ok((shaper, metrics, config));
    }
    core_text_layout(size, max_width, theme)
}

#[cfg(target_os = "macos")]
fn core_text_layout(
    size: f32,
    max_width: f32,
    theme: yu_core::ThemeId,
) -> Result<(CoreTextShaper, CoreTextViewportMetrics, LayoutConfig), i32> {
    if !size.is_finite() || size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    let request = FontRequest::new(theme.spec().body_font.family().unwrap_or("System UI"), size)
        .map_err(|_| YU_STORAGE_SHAPER_UNAVAILABLE)?;
    let shaper = if theme.spec().body_font == yu_core::ThemeFont::SystemUi {
        CoreTextShaper::from_system_ui(request)
    } else {
        CoreTextShaper::from_system(request)
    }
    .map_err(|_| YU_STORAGE_SHAPER_UNAVAILABLE)?;
    let metrics = shaper
        .viewport_metrics("M中🙂e\u{301}")
        .map_err(|_| YU_STORAGE_SHAPER_UNAVAILABLE)?;
    let config = LayoutConfig::new(max_width, metrics.line_height())
        .with_theme(theme)
        .with_base_direction(yu_core::BaseDirection::Ltr)
        .with_default_advance(metrics.default_advance());
    Ok((shaper, metrics, config))
}

#[cfg(target_os = "macos")]
fn caret_scroll_request_metadata(
    session: &DocumentEditorSession,
    request: CaretScrollRequest,
) -> Result<YuStorageCaretScrollRequest, i32> {
    let snapshot = session.snapshot();
    let caret = request.caret();
    let source_utf16 = snapshot
        .utf16_offset(caret.source())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let block_index = u64::try_from(caret.block()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    Ok(YuStorageCaretScrollRequest {
        revision: request.revision().get(),
        source_utf16,
        block_index,
        caret_x: caret.x(),
        caret_y: caret.y(),
        caret_width: caret.width(),
        caret_height: caret.height(),
        current_scroll_y: request.current_scroll_y(),
        target_scroll_y: request.target_scroll_y(),
        margin: request.margin(),
        needs_scroll: u8::from(request.needs_scroll()),
    })
}

fn composition_projection(session: &mut DocumentEditorSession) -> Result<VisualText, i32> {
    session.composition_visual_text().map_err(storage_status)
}

fn utf16_byte_offset(text: &str, target: u64) -> Result<usize, i32> {
    let target = usize::try_from(target).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    if target == 0 {
        return Ok(0);
    }
    let mut units = 0_usize;
    for (byte, character) in text.char_indices() {
        if units == target {
            return Ok(byte);
        }
        units = units.saturating_add(character.len_utf16());
        if units == target {
            return Ok(byte + character.len_utf8());
        }
        if units > target {
            return Err(YU_STORAGE_INVALID_SELECTION);
        }
    }
    if units == target {
        Ok(text.len())
    } else {
        Err(YU_STORAGE_INVALID_SELECTION)
    }
}

fn utf16_offset_in_utf8(text: &str, byte_offset: u64) -> Result<u64, i32> {
    let byte_offset = usize::try_from(byte_offset).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let prefix = text
        .get(..byte_offset)
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    u64::try_from(prefix.encode_utf16().count()).map_err(|_| YU_STORAGE_INVALID_SELECTION)
}

fn composition_visual_selection_utf16(
    projection: &VisualText,
    projected: &str,
) -> Result<(u64, u64), i32> {
    let visual_selection = projection
        .composition_selection_visual()
        .ok_or(YU_STORAGE_NO_OVERLAY)?;
    Ok((
        utf16_offset_in_utf8(projected, visual_selection.start().get())?,
        utf16_offset_in_utf8(projected, visual_selection.end().get())?,
    ))
}

fn composition_visual_replacement_utf16(
    projection: &VisualText,
    projected: &str,
) -> Result<(u64, u64), i32> {
    let replacement = projection
        .composition_range()
        .ok_or(YU_STORAGE_NO_OVERLAY)?;
    let visual_start = projection
        .source_to_visual(replacement.start(), Bias::Before)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let text_len = u64::try_from(
        projection
            .composition_text()
            .ok_or(YU_STORAGE_NO_OVERLAY)?
            .len(),
    )
    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let visual_end = visual_start
        .checked_add(text_len)
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    Ok((
        utf16_offset_in_utf8(projected, visual_start.get())?,
        utf16_offset_in_utf8(projected, visual_end.get())?,
    ))
}

fn composition_projection_metadata(
    session: &mut DocumentEditorSession,
) -> Result<(YuStorageCompositionProjection, String), i32> {
    let projection = composition_projection(session)?;
    let snapshot = session.snapshot();
    let overlay = session.composition().ok_or(YU_STORAGE_NO_OVERLAY)?;
    let projected = projected_utf8(&projection);
    let replacement_start_utf16 = snapshot
        .utf16_offset(overlay.replacement_range().start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let replacement_end_utf16 = snapshot
        .utf16_offset(overlay.replacement_range().end())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let (visual_selection_start_utf16, visual_selection_end_utf16) =
        composition_visual_selection_utf16(&projection, &projected)?;
    let (visual_replacement_start_utf16, visual_replacement_end_utf16) =
        composition_visual_replacement_utf16(&projection, &projected)?;
    let projected_utf16_length =
        u64::try_from(projected.encode_utf16().count()).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let projected_utf8_length =
        u64::try_from(projected.len()).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let metadata = YuStorageCompositionProjection {
        revision: snapshot.revision().get(),
        generation: session.composition_generation(),
        replacement_start_utf16,
        replacement_end_utf16,
        preedit_selection_start_utf16: overlay.selection_utf16().start().get(),
        preedit_selection_end_utf16: overlay.selection_utf16().end().get(),
        visual_selection_start_utf16,
        visual_selection_end_utf16,
        projected_utf16_length,
        projected_utf8_length,
        visual_replacement_start_utf16,
        visual_replacement_end_utf16,
    };
    Ok((metadata, projected))
}

#[cfg(target_os = "macos")]
fn composition_active_visual_caret(
    projection: &VisualText,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> Result<(VisualOffset, Bias), i32> {
    if selection_start_utf16 > selection_end_utf16 {
        return Err(YU_STORAGE_INVALID_SELECTION);
    }
    let text = projection.composition_text().ok_or(YU_STORAGE_NO_OVERLAY)?;
    let selection_start = utf16_byte_offset(text, selection_start_utf16)?;
    let selection_end = utf16_byte_offset(text, selection_end_utf16)?;
    let replacement = projection
        .composition_range()
        .ok_or(YU_STORAGE_NO_OVERLAY)?;
    let visual_base = projection
        .source_to_visual(replacement.start(), Bias::Before)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let active = if selection_start == selection_end {
        visual_base
            .checked_add(u64::try_from(selection_start).map_err(|_| YU_STORAGE_INVALID_SELECTION)?)
            .ok_or(YU_STORAGE_INVALID_SELECTION)?
    } else {
        visual_base
            .checked_add(u64::try_from(selection_end).map_err(|_| YU_STORAGE_INVALID_SELECTION)?)
            .ok_or(YU_STORAGE_INVALID_SELECTION)?
    };
    Ok((VisualOffset::new(active), Bias::After))
}

fn validate_revision(session: &DocumentEditorSession, expected: u64) -> Result<(), i32> {
    if session.revision().get() != expected {
        return Err(YU_STORAGE_STALE_REVISION);
    }
    Ok(())
}

fn table_resize_gesture_status(error: TableResizeGestureError) -> i32 {
    match error {
        TableResizeGestureError::StaleRevision { .. } => YU_STORAGE_STALE_REVISION,
        TableResizeGestureError::NonFinitePointer(_) => YU_STORAGE_INVALID_SELECTION,
    }
}

fn validate_table_resize_revision(
    session: &mut YuStorageSession,
    expected_revision: u64,
) -> Result<(), i32> {
    if session.session.revision().get() != expected_revision {
        session.table_resize_gesture = None;
        session.table_resize_override = None;
        return Err(YU_STORAGE_STALE_REVISION);
    }
    Ok(())
}

/// Returns the current selection's anchor/focus endpoints without losing
/// backward-drag direction. The ordered range remains available through
/// `yu_storage_session_selection` for callers that do not need direction.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_selection_endpoints(
    session: *const YuStorageSession,
    output: *mut YuStorageSelectionEndpoints,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    selection_endpoints_output(&session.session, output)
}

/// Sets a revision-bound selection while preserving anchor/focus direction.
/// This is used by shaped visual pointer drags; ordinary native selection
/// synchronization can continue using the ordered-range entry point above.
///
/// # Safety
/// `session` must be live. All UTF-16 offsets must belong to the expected
/// source revision and `affinity` must be a known value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_selection_endpoints(
    session: *mut YuStorageSession,
    expected_revision: u64,
    anchor_utf16: u64,
    focus_utf16: u64,
    affinity: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let selection =
        match selection_endpoints_from_ffi(&session.session, anchor_utf16, focus_utf16, affinity) {
            Ok(selection) => selection,
            Err(status) => return status,
        };
    session
        .session
        .set_selection(selection)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// Select an inclusive rectangular cell region through canonical source offsets.
/// # Safety
/// `session` must be a live handle. No source mutation is performed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_select_table_cells(
    session: *mut YuStorageSession,
    expected_revision: u64,
    anchor_utf16: u64,
    focus_utf16: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let endpoints = match selection_endpoints_from_ffi(
        &session.session,
        anchor_utf16,
        focus_utf16,
        YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
    ) {
        Ok(value) => value,
        Err(status) => return status,
    };
    session
        .session
        .execute(EditorCommand::SelectTableCells {
            anchor: endpoints.anchor(),
            focus: endpoints.focus(),
        })
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// Zero means text selection; a positive value is the selected cell column count.
/// # Safety
/// `session` must be live and `columns` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_selection_columns(
    session: *const YuStorageSession,
    expected_revision: u64,
    columns: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if columns.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    // SAFETY: caller supplies a writable output, checked non-null above.
    unsafe { *columns = session.session.selections().table_columns().unwrap_or(0) };
    YU_STORAGE_OK
}

/// 全部选区的端点，按文档顺序，外加主选区的下标。
///
/// 两遍协议（与 `outline_items` / `block_hidden_spans` 同一个）：`output` 传
/// null（`capacity` 为 0）时只把条数写进 `written`。
///
/// **单数入口 `yu_storage_session_selection_endpoints` 仍然留着**，它给的是
/// primary：AX 的 `AXSelectedTextRange`、滚动、「当前命中」这些按定义就只说
/// 得出一个位置的地方走那一个。复数走这里。
///
/// # Safety
/// `session` must be a live handle. `written` 与 `primary` must be writable;
/// `output` must provide `capacity` writable entries when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_selections(
    session: *const YuStorageSession,
    expected_revision: u64,
    output: *mut YuStorageSelectionEndpoints,
    capacity: usize,
    written: *mut usize,
    primary: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() || primary.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let selections = session.session.selections();
    let ranges = selections.as_slice();
    // SAFETY: both output pointers were checked above and belong to the caller.
    unsafe {
        *written = ranges.len();
        *primary = selections.primary_index();
    }
    if output.is_null() {
        return if capacity == 0 {
            YU_STORAGE_OK
        } else {
            YU_STORAGE_NULL_POINTER
        };
    }
    if capacity < ranges.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    let snapshot = session.session.snapshot();
    let revision = session.session.revision().get();
    let mut converted = Vec::with_capacity(ranges.len());
    for selection in ranges {
        let anchor_utf16 = match snapshot.utf16_offset(selection.anchor()) {
            Ok(offset) => offset.get(),
            Err(error) => {
                return status_from_editor_error(EditorDocumentError::Selection(error.into()));
            }
        };
        let focus_utf16 = match snapshot.utf16_offset(selection.focus()) {
            Ok(offset) => offset.get(),
            Err(error) => {
                return status_from_editor_error(EditorDocumentError::Selection(error.into()));
            }
        };
        converted.push(YuStorageSelectionEndpoints {
            revision,
            anchor_utf16,
            focus_utf16,
            affinity: match selection.affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        });
    }
    // SAFETY: capacity was checked against the converted length, and the
    // native caller supplied writable storage for that many values.
    unsafe {
        ptr::copy_nonoverlapping(converted.as_ptr(), output, converted.len());
    }
    YU_STORAGE_OK
}

/// 换一组选区。
///
/// 归一化（排序、合并、定位 primary）在 Rust 侧一家做——平台送来的可以是逆序
/// 的、重叠的、同一个偏移两次（⌥ 点在一段选区里就是），**这里不拒绝它们，
/// 归一化掉**。让平台先自己排一遍就是第二份合并实现，而两份合并必定分叉。
///
/// `count` 为 0 是错误：[`yu_editor::Selections`] 永远至少有一条选区，
/// 「一个光标都没有」在这个模型里不存在。要塌回一条走单数那个入口。**这条规则
/// 由 `Selections` 一家执行**，这一层不重复一遍。
///
/// # Safety
/// `session` must be live. `ranges` must provide `count` readable entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_selections(
    session: *mut YuStorageSession,
    expected_revision: u64,
    ranges: *const YuStorageSelectionEndpoints,
    count: usize,
    primary: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if ranges.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // **「至少一条」与「primary 在界内」不在这里再挡一遍。**
    //
    // 这两条是 `Selections::new` 的不变式，它已经挡住了（`count == 0` 与
    // `primary >= count` 都产出 `InvalidRange`，映射成同一个状态码）。写第二道
    // 只是给同一条规则加第二个答案，而且它永远不会被单独证伪——变异验证里把它
    // 改成 `if false` 之后全部用例照绿，正是这个意思。
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    // SAFETY: `ranges` is non-null and the caller guarantees `count` entries.
    let entries = unsafe { std::slice::from_raw_parts(ranges, count) };
    let mut selections = Vec::with_capacity(count);
    for entry in entries {
        match selection_endpoints_from_ffi(
            &session.session,
            entry.anchor_utf16,
            entry.focus_utf16,
            entry.affinity,
        ) {
            Ok(selection) => selections.push(selection),
            Err(status) => return status,
        }
    }
    session
        .session
        .set_selections(selections, primary)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_execute_command(
    session: *mut YuStorageSession,
    command: u8,
    block: u64,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let command = match command_from_ffi(command, block) {
        Ok(command) => command,
        Err(status) => return status,
    };
    let result = match session.session.execute(command) {
        Ok(result) => result,
        Err(error) => return storage_status(error),
    };
    command_result_output(&session.session, result, output)
}

/// Executes one vertical caret command against the current shaped
/// block layouts. The command result still uses the ordinary source/selection
/// contract; the platform shaper is only a layout provider for this query.
/// The host must have published matching viewport metrics first.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_move_vertical(
    session: *mut YuStorageSession,
    expected_revision: u64,
    command: u8,
    size: f32,
    max_width: f32,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageCommandResult::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session, expected_revision, command, size, max_width);
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        let (up, extend) = match command {
            YU_STORAGE_COMMAND_MOVE_UP => (true, false),
            YU_STORAGE_COMMAND_MOVE_DOWN => (false, false),
            YU_STORAGE_COMMAND_MOVE_UP_EXTEND => (true, true),
            YU_STORAGE_COMMAND_MOVE_DOWN_EXTEND => (false, true),
            _ => return YU_STORAGE_INVALID_COMMAND,
        };
        let (shaper, metrics, config) = match macos_query_text_layout(
            session,
            size,
            max_width,
            session.session.viewport_config().layout().theme(),
        ) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(
            session,
            max_width,
            metrics,
            session.session.viewport_config().layout().theme(),
        ) {
            return status;
        }
        let result = match session
            .session
            .move_vertical_with_shaper(up, extend, config, &shaper)
        {
            Ok(result) => result,
            Err(error) => return storage_status(error),
        };
        command_result_output(&session.session, result, output)
    }
}

/// Inserts permanent text through the same editor command path used by all
/// other native edits. `expected_revision` prevents an NSTextInputClient
/// callback queued for an older source mirror from applying late.
///
/// # Safety
/// `session` must be a live handle. `text` must point to a readable UTF-8
/// buffer of `text_length` bytes, and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_insert_text(
    session: *mut YuStorageSession,
    expected_revision: u64,
    text: *const u8,
    text_length: usize,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let text = match read_utf8(text, text_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let result = match session.session.execute(EditorCommand::insert_text(text)) {
        Ok(result) => result,
        Err(error) => return storage_status(error),
    };
    command_result_output(&session.session, result, output)
}

/// UTF-8 byte ranges in the batch's shared input buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct YuStorageImageInput {
    pub path_start: usize,
    pub path_end: usize,
    pub alternative_start: usize,
    pub alternative_end: usize,
}

/// Insert an entire imported batch in one source/undo transaction. The native
/// host stages resources first and discards unpublished copies on failure.
/// # Safety
/// Session must be live, text/items readable for their lengths, output writable.
/// A non-null drop_target must point to a readable revision-bound caret.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yu_storage_session_insert_local_images(
    session: *mut YuStorageSession,
    expected_revision: u64,
    text: *const u8,
    text_length: usize,
    items: *const YuStorageImageInput,
    count: usize,
    drop_target: *const YuStorageSelectionEndpoints,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() || items.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if count == 0 || count > isize::MAX as usize / std::mem::size_of::<YuStorageImageInput>() {
        return YU_STORAGE_INVALID_SELECTION;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let drop_offset = if let Some(target) = unsafe { drop_target.as_ref() } {
        if target.revision != expected_revision {
            return YU_STORAGE_STALE_REVISION;
        }
        if target.anchor_utf16 != target.focus_utf16 {
            return YU_STORAGE_INVALID_SELECTION;
        }
        match selection_endpoints_from_ffi(
            &session.session,
            target.anchor_utf16,
            target.focus_utf16,
            target.affinity,
        ) {
            Ok(selection) => Some(selection.focus()),
            Err(status) => return status,
        }
    } else {
        None
    };
    let text = match read_utf8(text, text_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    // SAFETY: count/nonnull checked above; caller supplies readable records.
    let items = unsafe { std::slice::from_raw_parts(items, count) };
    let mut images = Vec::with_capacity(count);
    for item in items {
        let (Some(path), Some(alternative)) = (
            text.get(item.path_start..item.path_end),
            text.get(item.alternative_start..item.alternative_end),
        ) else {
            return YU_STORAGE_INVALID_SELECTION;
        };
        if yu_editor::local_image_uri(path).is_none() {
            return YU_STORAGE_INVALID_PATH;
        }
        images.push((path, alternative));
    }
    match session
        .session
        .document_mut()
        .editor_mut()
        .insert_local_images(&images, drop_offset)
    {
        Ok(result) => command_result_output(&session.session, result, output),
        Err(error) => status_from_editor_error(error),
    }
}

/// Encode a local path for an editable image URI field.
/// # Safety
/// Path must be readable for length bytes; output and written follow the
/// ordinary UTF-8 two-call copy contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_encode_local_image_path(
    path: *const u8,
    length: usize,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let path = match read_utf8(path, length) {
        Ok(path) => path,
        Err(status) => return status,
    };
    let Some(uri) = yu_editor::local_image_uri(path) else {
        return YU_STORAGE_INVALID_PATH;
    };
    write_bytes(uri.as_bytes(), output, capacity, written)
}

/// Source identity and declared dimensions. Zero means intrinsic dimension.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct YuStorageImageProperties {
    pub revision: u64,
    pub start_utf16: u64,
    pub end_utf16: u64,
    pub width: u32,
    pub height: u32,
}

fn image_properties_from_ffi(
    session: &DocumentEditorSession,
    info: &YuStorageImageProperties,
) -> Result<yu_editor::ImageProperties, i32> {
    validate_revision(session, info.revision)?;
    let range = source_range_from_ffi(session, info.start_utf16, info.end_utf16)?;
    session
        .document()
        .editor()
        .image_properties(range)
        .ok_or(YU_STORAGE_INVALID_SELECTION)
}

/// Inspect the image containing a source caret. At an adjacent boundary the
/// image starting at that boundary wins. Hosts can use the exact image-hit start.
/// # Safety
/// Session must be live and output must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_image_properties(
    session: *const YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    output: *mut YuStorageImageProperties,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let snapshot = session.session.snapshot();
    let offset = match snapshot.byte_offset_for_utf16(Utf16Offset::new(source_utf16)) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let editor = session.session.document().editor();
    let images = match editor.image_references() {
        Ok(images) => images,
        Err(error) => return status_from_editor_error(error),
    };
    let image = images
        .iter()
        .find(|image| image.source.start() <= offset && offset < image.source.end())
        .or_else(|| images.iter().find(|image| image.source.end() == offset));
    let Some(properties) = image.and_then(|image| editor.image_properties(image.source)) else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let (start_utf16, end_utf16) = match source_utf16_range(&snapshot, properties.source) {
        Ok(range) => range,
        Err(status) => return status,
    };
    unsafe {
        *output = YuStorageImageProperties {
            revision: expected_revision,
            start_utf16,
            end_utf16,
            width: properties.width.unwrap_or(0),
            height: properties.height.unwrap_or(0),
        };
    }
    YU_STORAGE_OK
}

/// Query the resource state of an exact, revision-bound image identity.
/// # Safety
/// Session and info must be live; output must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_image_resource_status(
    session: *const YuStorageSession,
    info: *const YuStorageImageProperties,
    output: *mut u8,
) -> i32 {
    let (Some(session), Some(info)) = (unsafe { session.as_ref() }, unsafe { info.as_ref() })
    else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let properties = match image_properties_from_ffi(&session.session, info) {
        Ok(value) => value,
        Err(status) => return status,
    };
    #[cfg(target_os = "macos")]
    {
        let key = ImageKey::new(properties.destination).ok();
        unsafe {
            *output = macos_image_resource_status(
                session.macos_render_host.as_ref(),
                key.as_ref(),
                info.revision,
            );
        }
        YU_STORAGE_OK
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = properties;
        YU_STORAGE_RENDER_HOST_UNAVAILABLE
    }
}

/// Reset a failed image's retry budget without editing source or history.
/// The next surface submission schedules decoding. A running request is not
/// duplicated, and a stale identity cannot reset another document's failure.
/// # Safety
/// Session and info must be live, used on the session's owning thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_retry_image(
    session: *mut YuStorageSession,
    info: *const YuStorageImageProperties,
) -> i32 {
    let (Some(session), Some(info)) = (unsafe { session.as_mut() }, unsafe { info.as_ref() })
    else {
        return YU_STORAGE_NULL_POINTER;
    };
    let properties = match image_properties_from_ffi(&session.session, info) {
        Ok(value) => value,
        Err(status) => return status,
    };
    if session.session.composition().is_some() {
        return YU_STORAGE_INVALID_SELECTION;
    }
    #[cfg(target_os = "macos")]
    {
        let key = match ImageKey::new(properties.destination) {
            Ok(key) => key,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let Some(state) = session.macos_render_host.as_mut() else {
            return YU_STORAGE_RENDER_HOST_UNAVAILABLE;
        };
        if !state.image_resources.in_flight.contains(&key)
            && state
                .image_resources
                .cache
                .retry_failure(&key, session.session.revision())
        {
            if let Some(worker) = state.frame_worker.as_ref() {
                worker.invalidate();
            }
            state.frame_worker_request = None;
            state.last_frame_key = None;
            state.prepared_build = None;
        }
        YU_STORAGE_OK
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = properties;
        YU_STORAGE_RENDER_HOST_UNAVAILABLE
    }
}

/// Copy source-backed prose intervals for native spelling. Null output queries count.
/// # Safety
/// Session must be live; written writable; output writable for capacity records.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_spelling_ranges(
    session: *const YuStorageSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    output: *mut YuStorageAccessibilityRange,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let range = match source_range_from_ffi(&session.session, start_utf16, end_utf16) {
        Ok(v) => v,
        Err(s) => return s,
    };
    let snapshot = session.session.snapshot();
    let ranges = if session.session.composition().is_some() {
        Vec::new()
    } else {
        session
            .session
            .document()
            .editor()
            .markdown()
            .spelling_ranges(range)
    };
    let mut records = Vec::with_capacity(ranges.len());
    for range in ranges {
        let (start_utf16, end_utf16) = match source_utf16_range(&snapshot, range) {
            Ok(v) => v,
            Err(s) => return s,
        };
        records.push(YuStorageAccessibilityRange {
            revision: expected_revision,
            start_utf16,
            end_utf16,
        });
    }
    unsafe {
        *written = records.len();
    }
    if output.is_null() {
        return YU_STORAGE_OK;
    }
    if capacity < records.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    unsafe {
        ptr::copy_nonoverlapping(records.as_ptr(), output, records.len());
    }
    YU_STORAGE_OK
}

/// Query a visible details header. An empty range means no disclosure target.
/// # Safety
/// Session must be live and both outputs writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_disclosure_header(
    session: *const YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    output: *mut YuStorageAccessibilityRange,
    open: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() || open.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let at = match source_range_from_ffi(&session.session, source_utf16, source_utf16) {
        Ok(range) => range.start(),
        Err(status) => return status,
    };
    let details = session
        .session
        .document()
        .editor()
        .html_disclosure_header_at(at);
    let (start_utf16, end_utf16, expanded) = if let Some(details) = details {
        let header = TextRange::new(
            details.opening.start(),
            details
                .summary
                .map_or(details.opening.end(), |summary| summary.end()),
        )
        .expect("ordered disclosure header");
        match source_utf16_range(&session.session.snapshot(), header) {
            Ok((start, end)) => (start, end, u8::from(details.open)),
            Err(status) => return status,
        }
    } else {
        (0, 0, 0)
    };
    unsafe {
        *output = YuStorageAccessibilityRange {
            revision: expected_revision,
            start_utf16,
            end_utf16,
        };
        *open = expanded;
    }
    YU_STORAGE_OK
}

/// Reveal a navigation target without modifying source or undo history.
/// # Safety
/// Session must be live; changed must be writable. Ranges are canonical UTF-16.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_reveal_source_range(
    session: *mut YuStorageSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    changed: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if changed.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let range = match source_range_from_ffi(&session.session, start_utf16, end_utf16) {
        Ok(range) => range,
        Err(status) => return status,
    };
    match session
        .session
        .document_mut()
        .editor_mut()
        .reveal_html_range(range)
    {
        Ok(revealed) => {
            unsafe {
                *changed = u8::from(revealed);
            }
            if revealed {
                invalidate_disclosure_frame(session);
            }
            YU_STORAGE_OK
        }
        Err(_) => YU_STORAGE_EDITOR_ERROR,
    }
}

fn invalidate_disclosure_frame(_session: &mut YuStorageSession) {
    #[cfg(target_os = "macos")]
    if let Some(state) = _session.macos_render_host.as_mut() {
        if let Some(worker) = state.frame_worker.as_ref() {
            worker.invalidate();
        }
        state.frame_worker_request = None;
        state.last_frame_key = None;
        state.prepared_build = None;
        // Collapsing changes block indices without changing source revision.
        // Never feed the previous visible block indices into resource planning.
        state.visibility_revision = None;
        state.visible_blocks.clear();
    }
}

/// Toggle a visible details header through the canonical session/history path.
/// # Safety
/// Session must be live and output writable. Position is canonical UTF-16.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_toggle_disclosure(
    session: *mut YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let source = match source_range_from_ffi(&session.session, source_utf16, source_utf16) {
        Ok(range) => range.start(),
        Err(status) => return status,
    };
    match session
        .session
        .execute(EditorCommand::ToggleHtmlDetails { source })
    {
        Ok(result) => {
            invalidate_disclosure_frame(session);
            command_result_output(&session.session, result, output)
        }
        Err(error) => storage_status(error),
    }
}

/// Copy the decoded link destination at a canonical UTF-16 label position.
/// Empty output means no link. This function never opens a URL.
/// # Safety
/// Session must be live; output/written follow the standard copy contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_link_destination(
    session: *const YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let at = match source_range_from_ffi(&session.session, source_utf16, source_utf16) {
        Ok(range) => range.start(),
        Err(status) => return status,
    };
    let link = session.session.document().editor().markdown().link_at(at);
    let destination = link.as_ref().map_or("", |link| link.destination.as_str());
    write_bytes(destination.as_bytes(), output, capacity, written)
}

/// Query an equation or footnote reference/backlink target in canonical UTF-16 coordinates.
/// An empty output range means no valid target exists.
/// # Safety
/// Session must be live and output writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_document_reference_target(
    session: *const YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    output: *mut YuStorageAccessibilityRange,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let at = match source_range_from_ffi(&session.session, source_utf16, source_utf16) {
        Ok(range) => range.start(),
        Err(status) => return status,
    };
    let markdown = session.session.document().editor().markdown();
    let target = markdown
        .footnotes()
        .navigation_target(at)
        .or_else(|| markdown.equation_reference_target(at))
        .or_else(|| {
            markdown.link_at(at).and_then(|link| {
                yu_editor::document_anchor_target(markdown, &link.destination)
                    .ok()
                    .flatten()
            })
        });
    let (start_utf16, end_utf16) = match target {
        Some(target) => match source_utf16_range(&session.session.snapshot(), target) {
            Ok(range) => range,
            Err(status) => return status,
        },
        None => (0, 0),
    };
    unsafe {
        *output = YuStorageAccessibilityRange {
            revision: expected_revision,
            start_utf16,
            end_utf16,
        };
    }
    YU_STORAGE_OK
}

/// Read a current document diagnostic at a UTF-16 source position.
/// An empty string means there is no diagnostic at that position.
/// # Safety
/// Session must be live; output/written must follow the standard copy contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_document_diagnostic(
    session: *const YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let position = match source_range_from_ffi(&session.session, source_utf16, source_utf16) {
        Ok(range) => range.start(),
        Err(status) => return status,
    };
    #[cfg(target_os = "macos")]
    let message = session
        .macos_embedded_resources
        .diagnostics
        .iter()
        .find(|(revision, range, _)| {
            revision.get() == expected_revision
                && range.start() <= position
                && position < range.end()
        })
        .map_or("", |(_, _, message)| message.as_str());
    #[cfg(not(target_os = "macos"))]
    let message = {
        let _ = position;
        ""
    };
    let message = if message.is_empty() {
        session
            .session
            .document()
            .editor()
            .markdown()
            .footnotes()
            .diagnostic_at(position)
            .unwrap_or("")
    } else {
        message
    };
    write_bytes(message.as_bytes(), output, capacity, written)
}

/// Publish one complete current diagnostic set. Empty input clears decorations.
/// # Safety
/// Session must be live; records readable for count elements when nonzero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_spelling_diagnostics(
    session: *mut YuStorageSession,
    expected_revision: u64,
    records: *const YuStorageAccessibilityRange,
    count: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    if count > 4096 {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    if count > 0 && records.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let records = if count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(records, count) }
    };
    let mut ranges = Vec::with_capacity(count);
    for record in records {
        if record.revision != expected_revision {
            return YU_STORAGE_STALE_REVISION;
        }
        match source_range_from_ffi(&session.session, record.start_utf16, record.end_utf16) {
            Ok(range) => ranges.push(range),
            Err(status) => return status,
        }
    }
    let revision = session.session.revision();
    match session
        .session
        .document_mut()
        .editor_mut()
        .set_spelling_diagnostics(revision, ranges)
    {
        Ok(()) => YU_STORAGE_OK,
        Err(error) => status_from_editor_error(error),
    }
}

/// Replace one current prose diagnostic with a separate undo step.
/// # Safety
/// Session/info and replacement must be readable; output writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_replace_spelling(
    session: *mut YuStorageSession,
    info: *const YuStorageAccessibilityRange,
    replacement: *const u8,
    length: usize,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let (Some(session), Some(info)) = (unsafe { session.as_mut() }, unsafe { info.as_ref() })
    else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, info.revision) {
        return status;
    }
    let range = match source_range_from_ffi(&session.session, info.start_utf16, info.end_utf16) {
        Ok(v) => v,
        Err(s) => return s,
    };
    let text = match read_utf8(replacement, length) {
        Ok(v) => v,
        Err(s) => return s,
    };
    match session
        .session
        .document_mut()
        .editor_mut()
        .replace_spelling(range, text)
    {
        Ok(result) => command_result_output(&session.session, result, output),
        Err(error) => status_from_editor_error(error),
    }
}

/// Copy destination URI (field 0) or alternative text (field 1).
/// # Safety
/// Session/info must be readable, written writable, output writable for capacity.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_image_property(
    session: *const YuStorageSession,
    info: *const YuStorageImageProperties,
    field: u8,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let (Some(session), Some(info)) = (unsafe { session.as_ref() }, unsafe { info.as_ref() })
    else {
        return YU_STORAGE_NULL_POINTER;
    };
    let properties = match image_properties_from_ffi(&session.session, info) {
        Ok(value) => value,
        Err(status) => return status,
    };
    let value = match field {
        0 => properties.destination,
        1 => properties.alternative,
        _ => return YU_STORAGE_INVALID_COMMAND,
    };
    write_bytes(value.as_bytes(), output, capacity, written)
}

/// Update an exact image in one isolated undo transaction. Width/height are
/// declared dimensions; zero removes that dimension. Destination is a URI.
/// # Safety
/// Session must be live, info/buffers readable, output writable.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yu_storage_session_update_image_properties(
    session: *mut YuStorageSession,
    info: *const YuStorageImageProperties,
    destination: *const u8,
    destination_length: usize,
    alternative: *const u8,
    alternative_length: usize,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let (Some(session), Some(info)) = (unsafe { session.as_mut() }, unsafe { info.as_ref() })
    else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let mut properties = match image_properties_from_ffi(&session.session, info) {
        Ok(value) => value,
        Err(status) => return status,
    };
    properties.destination = match read_utf8(destination, destination_length) {
        Ok(value) => value.into(),
        Err(status) => return status,
    };
    properties.alternative = match read_utf8(alternative, alternative_length) {
        Ok(value) => value.into(),
        Err(status) => return status,
    };
    properties.width = (info.width != 0).then_some(info.width);
    properties.height = (info.height != 0).then_some(info.height);
    match session
        .session
        .document_mut()
        .editor_mut()
        .update_image_properties(&properties)
    {
        Ok(result) => command_result_output(&session.session, result, output),
        Err(error) => status_from_editor_error(error),
    }
}

pub const YU_STORAGE_PASTE_PLAIN_TEXT: u8 = 0;
pub const YU_STORAGE_PASTE_TABULAR_TEXT: u8 = 1;

/// Paste external text, retaining its explicit tabular/ordinary representation.
/// # Safety
/// Session must be live, text readable for text_length bytes, output writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_paste_text(
    session: *mut YuStorageSession,
    expected_revision: u64,
    text: *const u8,
    text_length: usize,
    format: u8,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if !matches!(
        format,
        YU_STORAGE_PASTE_PLAIN_TEXT | YU_STORAGE_PASTE_TABULAR_TEXT
    ) {
        return YU_STORAGE_INVALID_COMMAND;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let text = match read_utf8(text, text_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let command = EditorCommand::PasteClipboardText {
        text: text.into(),
        tabular: format == YU_STORAGE_PASTE_TABULAR_TEXT,
    };
    match session.session.execute(command) {
        Ok(result) => command_result_output(&session.session, result, output),
        Err(error) => storage_status(error),
    }
}

/// Paste UTF-8 fragments stored contiguously in `text`. Each cumulative end
/// offset must be on a UTF-8 boundary; the final end must equal text_length.
/// Validation completes before executing the single atomic editor command.
///
/// # Safety
/// `session` must be live. `text` must provide `text_length` readable bytes,
/// `ends` must provide `count` readable size_t entries, and `output` must be
/// writable. Empty buffers may be null.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_paste_fragments(
    session: *mut YuStorageSession,
    expected_revision: u64,
    text: *const u8,
    text_length: usize,
    ends: *const usize,
    count: usize,
    columns: usize,
    format: u8,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() || (count > 0 && ends.is_null()) {
        return YU_STORAGE_NULL_POINTER;
    }
    if count > isize::MAX as usize / std::mem::size_of::<usize>() {
        return YU_STORAGE_INVALID_SELECTION;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    if format > YU_STORAGE_FRAGMENT_HTML_TABLE
        || (matches!(
            format,
            YU_STORAGE_FRAGMENT_MARKDOWN | YU_STORAGE_FRAGMENT_HTML
        ) && columns == 0)
        || (format == YU_STORAGE_FRAGMENT_HTML_TABLE && (columns != 0 || count != 1))
    {
        return YU_STORAGE_INVALID_COMMAND;
    }
    let text = match read_utf8(text, text_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let ends = if count == 0 {
        &[][..]
    } else {
        // SAFETY: the caller provides count readable entries; null and size
        // checks above satisfy slice construction requirements.
        unsafe { std::slice::from_raw_parts(ends, count) }
    };
    let mut fragments = Vec::with_capacity(count);
    let mut start = 0;
    for &end in ends {
        let Some(fragment) = text.get(start..end) else {
            return YU_STORAGE_INVALID_SELECTION;
        };
        fragments.push(std::sync::Arc::from(fragment));
        start = end;
    }
    if start != text.len() {
        return YU_STORAGE_INVALID_SELECTION;
    }
    if format == YU_STORAGE_FRAGMENT_HTML
        && session
            .session
            .document()
            .editor()
            .html_grid_source(columns, &fragments)
            .is_err()
    {
        return YU_STORAGE_INVALID_SELECTION;
    }
    let destination = session.session.document().editor().table_target_is_html();
    let html_grid = columns > 0
        && ((format == YU_STORAGE_FRAGMENT_HTML && destination != Some(false))
            || (format == YU_STORAGE_FRAGMENT_MARKDOWN && destination == Some(true)));
    if format == YU_STORAGE_FRAGMENT_MARKDOWN && destination == Some(true) {
        fragments = fragments
            .iter()
            .map(|cell| std::sync::Arc::from(yu_export::export_html_fragment(cell)))
            .collect();
    } else if format == YU_STORAGE_FRAGMENT_HTML && destination == Some(false) {
        let converted = fragments
            .iter()
            .map(|cell| {
                yu_export::import_html_fragment(cell).map(|text| {
                    std::sync::Arc::from(
                        text.trim_end_matches(['\r', '\n'])
                            .replace("\r\n", "\n")
                            .replace('\n', "<br>"),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>();
        fragments = match converted {
            Ok(cells) => cells,
            Err(_) => return YU_STORAGE_INVALID_COMMAND,
        };
    }
    let command = if format == YU_STORAGE_FRAGMENT_HTML_TABLE {
        EditorCommand::PasteHtmlTableSource(fragments[0].clone())
    } else if html_grid {
        EditorCommand::PasteHtmlTableGrid {
            columns,
            cells: fragments,
        }
    } else if columns == 0 {
        EditorCommand::PasteFragments(fragments)
    } else {
        EditorCommand::PasteTableGrid {
            columns,
            cells: fragments,
        }
    };
    let result = match session.session.execute(command) {
        Ok(result) => result,
        Err(error) => return storage_status(error),
    };
    command_result_output(&session.session, result, output)
}

/// # Safety
///
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_command_available(
    session: *const YuStorageSession,
    command: u8,
    block: u64,
    output: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let command = match command_from_ffi(command, block) {
        Ok(command) => command,
        Err(status) => return status,
    };
    // SAFETY: output was checked above and belongs to the caller.
    unsafe {
        *output = u8::from(session.session.command_available(&command));
    }
    YU_STORAGE_OK
}

/// Returns the current transient composition metadata. The source revision
/// remains unchanged while `active` is non-zero; `generation` changes on each
/// successful begin/update/commit/cancel transition.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_composition(
    session: *const YuStorageSession,
    output: *mut YuStorageCompositionState,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let mut state = YuStorageCompositionState {
        revision: session.session.revision().get(),
        generation: session.session.composition_generation(),
        ..YuStorageCompositionState::default()
    };
    if let Some(overlay) = session.session.composition() {
        let snapshot = session.session.snapshot();
        let replacement = overlay.replacement_range();
        let start = match snapshot.utf16_offset(replacement.start()) {
            Ok(value) => value,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let end = match snapshot.utf16_offset(replacement.end()) {
            Ok(value) => value,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let selection = overlay.selection_utf16();
        state.replacement_start_utf16 = start.get();
        state.replacement_end_utf16 = end.get();
        state.selection_start_utf16 = selection.start().get();
        state.selection_end_utf16 = selection.end().get();
        state.preedit_utf8_length = overlay.text().len() as u64;
        state.active = 1;
    }
    // SAFETY: output was checked above and belongs to the caller.
    unsafe { *output = state };
    YU_STORAGE_OK
}

/// Copies the active preedit after validating both source revision and
/// composition generation. It uses the same two-call length/copy convention
/// as the source snapshot functions.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_composition(
    session: *const YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let Some(overlay) = session.session.composition() else {
        return YU_STORAGE_NO_OVERLAY;
    };
    write_bytes(overlay.text().as_bytes(), output, capacity, written)
}

/// # Safety
///
/// `session` must be a live handle. `preedit` must point to readable UTF-8
/// bytes for the given length unless the length is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_begin_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    replacement_start_utf16: u64,
    replacement_end_utf16: u64,
    preedit: *const u8,
    preedit_length: usize,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let preedit = match read_utf8(preedit, preedit_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let replacement = match source_range_from_ffi(
        &session.session,
        replacement_start_utf16,
        replacement_end_utf16,
    ) {
        Ok(range) => range,
        Err(status) => return status,
    };
    let selection = match Utf16Range::new(
        Utf16Offset::new(selection_start_utf16),
        Utf16Offset::new(selection_end_utf16),
    ) {
        Some(selection) => selection,
        None => return YU_STORAGE_INVALID_SELECTION,
    };
    session
        .session
        .begin_composition(replacement, preedit, selection)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle. `preedit` must point to readable UTF-8
/// bytes for the given length unless the length is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_update_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    preedit: *const u8,
    preedit_length: usize,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let preedit = match read_utf8(preedit, preedit_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    let selection = match Utf16Range::new(
        Utf16Offset::new(selection_start_utf16),
        Utf16Offset::new(selection_end_utf16),
    ) {
        Some(selection) => selection,
        None => return YU_STORAGE_INVALID_SELECTION,
    };
    session
        .session
        .update_composition(preedit, selection)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle. `committed_text` must point to readable
/// UTF-8 bytes for the given length unless the length is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_commit_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    committed_text: *const u8,
    committed_length: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let committed_text = match read_utf8(committed_text, committed_length) {
        Ok(text) => text,
        Err(status) => return status,
    };
    session
        .session
        .commit_composition(committed_text)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
}

/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_cancel_composition(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let _ = session.session.cancel_composition();
    YU_STORAGE_OK
}

/// # Safety
///
/// `output` must be writable for one session pointer. When `path_length` is
/// non-zero, `path` must point to a readable UTF-8 byte buffer of that size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_open(
    path: *const u8,
    path_length: usize,
    output: *mut *mut YuStorageSession,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `output` is a caller-owned pointer checked above.
    unsafe { *output = ptr::null_mut() };
    let path = match read_utf8(path, path_length) {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    let session = match DocumentEditorSession::open(path) {
        Ok(session) => session,
        Err(error) => return status_from_error(error),
    };
    let session = new_storage_session(session);
    // SAFETY: the pointer is transferred to the native caller as an opaque
    // handle and is reclaimed only by `yu_storage_session_destroy`.
    unsafe { *output = Box::into_raw(session) };
    YU_STORAGE_OK
}

fn new_storage_session(session: DocumentEditorSession) -> Box<YuStorageSession> {
    Box::new(YuStorageSession {
        focus_mode: false,
        session,
        table_resize_gesture: None,
        table_resize_override: None,
        #[cfg(target_os = "macos")]
        macos_render_host: None,
        #[cfg(target_os = "macos")]
        shader_library: None,
        #[cfg(target_os = "macos")]
        macos_embedded_resources: MacosEmbeddedResourceState::new(),
    })
}

/// Creates an unsaved editor without creating a document file.
/// # Safety
/// `path` must contain readable UTF-8 bytes; `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_create(
    path: *const u8,
    path_length: usize,
    output: *mut *mut YuStorageSession,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    unsafe { *output = ptr::null_mut() };
    let path = match read_utf8(path, path_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    unsafe { *output = Box::into_raw(new_storage_session(DocumentEditorSession::new(path, ""))) };
    YU_STORAGE_OK
}

/// Restores source and the pre-crash disk baseline. Does not write the target.
/// # Safety
/// `path` must contain readable UTF-8 bytes; `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_open_recovery(
    path: *const u8,
    path_length: usize,
    output: *mut *mut YuStorageSession,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    unsafe { *output = ptr::null_mut() };
    let path = match read_utf8(path, path_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    let record = match RecoveryRecord::read(&path) {
        Ok(record) => record,
        Err(_) => return YU_STORAGE_INVALID_RECOVERY,
    };
    unsafe { *output = Box::into_raw(new_storage_session(DocumentEditorSession::recover(record))) };
    YU_STORAGE_OK
}

/// Reads discovery metadata without interpreting the envelope in Swift.
/// # Safety
/// Input bytes must be readable; outputs follow the standard count/fill contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_recovery_copy_target(
    path: *const u8,
    path_length: usize,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let path = match read_utf8(path, path_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    let record = match RecoveryRecord::read(&path) {
        Ok(record) => record,
        Err(_) => return YU_STORAGE_INVALID_RECOVERY,
    };
    let Some(target) = record.target_path().to_str() else {
        return YU_STORAGE_INVALID_PATH;
    };
    write_bytes(target.as_bytes(), output, capacity, written)
}

/// Writes (action 0) or clears (action 1) this session's recovery record.
/// # Safety
/// `session` must be live and root must contain readable UTF-8 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_recovery(
    session: *const YuStorageSession,
    root: *const u8,
    root_length: usize,
    action: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let root = match read_utf8(root, root_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    let store = RecoveryStore::new(root);
    let result = match action {
        0 => session.session.write_recovery(&store).map(|_| ()),
        1 => store.clear(session.session.path()),
        _ => return YU_STORAGE_INVALID_COMMAND,
    };
    result.map_or(YU_STORAGE_IO_ERROR, |()| YU_STORAGE_OK)
}

/// Resolves a recovery envelope path without opening or parsing a document.
/// # Safety
/// Readable root/target UTF-8 bytes and standard count/fill output pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_recovery_copy_path(
    root: *const u8,
    root_length: usize,
    target: *const u8,
    target_length: usize,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let root = match read_utf8(root, root_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    let target = match read_utf8(target, target_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    let path = match RecoveryStore::new(root).path_for(&target) {
        Ok(path) => path,
        Err(_) => return YU_STORAGE_INVALID_PATH,
    };
    let Some(path) = path.to_str() else {
        return YU_STORAGE_INVALID_PATH;
    };
    write_bytes(path.as_bytes(), output, capacity, written)
}

/// Save As keeps the canonical document and history; invalidates path-relative resources.
/// # Safety
/// Live session, called on the host thread; readable destination path bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_save_as(
    session: *mut YuStorageSession,
    path: *const u8,
    path_length: usize,
    replace_existing: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if replace_existing > 1 {
        return YU_STORAGE_INVALID_COMMAND;
    }
    if session.session.composition().is_some() {
        return YU_STORAGE_INVALID_STATE;
    }
    let path = match read_utf8(path, path_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    if let Err(error) = session.session.save_as(path, replace_existing != 0) {
        return status_from_error(error);
    }
    session.table_resize_gesture = None;
    session.table_resize_override = None;
    #[cfg(target_os = "macos")]
    {
        session.macos_render_host = None;
        session.macos_embedded_resources = MacosEmbeddedResourceState::new();
    }
    YU_STORAGE_OK
}

/// Configure application-owned column metadata storage for this session.
/// Malformed or stale cache entries are ignored by the store; I/O errors surface.
/// # Safety
/// `session` must be live; `root` must name readable UTF-8 bytes for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_table_width_store(
    session: *mut YuStorageSession,
    root: *const u8,
    root_length: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let root = match read_utf8(root, root_length) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => return YU_STORAGE_INVALID_PATH,
        Err(status) => return status,
    };
    match session.session.document_mut().set_table_width_store(root) {
        Ok(_) => YU_STORAGE_OK,
        Err(error) => status_from_error(error),
    }
}

/// Persist committed column widths independently from text dirty state.
/// Dirty source is deferred until the next successful save.
/// # Safety
/// `session` must be null or a live session handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_persist_table_widths(
    session: *mut YuStorageSession,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    match session.session.document().persist_table_widths() {
        Ok(yu_storage::TableWidthStoreOutcome::Stale) => YU_STORAGE_EXTERNAL_CHANGE,
        Ok(_) => YU_STORAGE_OK,
        Err(error) => status_from_error(error),
    }
}

/// # Safety
///
/// `session` must be null or a live handle returned by
/// `yu_storage_session_open`, and must not be destroyed more than once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_destroy(session: *mut YuStorageSession) {
    if !session.is_null() {
        // SAFETY: the handle came from `yu_storage_session_open` and is
        // destroyed at most once by the caller.
        unsafe { drop(Box::from_raw(session)) };
    }
}

/// # Safety
///
/// `session` must be null or a live handle. `output`/`written` must describe a
/// valid writable buffer, except for the documented null/zero-capacity query.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_path(
    session: *const YuStorageSession,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let path = session.session.path().to_string_lossy();
    write_bytes(path.as_bytes(), output, capacity, written)
}

/// # Safety
///
/// `session` must be null or a live handle. `output`/`written` must describe a
/// valid writable buffer, except for the documented null/zero-capacity query.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_source(
    session: *const YuStorageSession,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let source = session.session.snapshot();
    write_bytes(source.as_str().as_bytes(), output, capacity, written)
}

/// Maps one canonical source caret through the current inline projection.
/// `visual_utf16` and `round_trip_source_utf16` are both bound to
/// `expected_revision`; hidden delimiter affinity is controlled by the same
/// upstream/downstream values used by the editor selection contract.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projection_caret(
    session: *mut YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    affinity: u8,
    output: *mut YuStorageProjectionCaret,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let affinity = match caret_affinity_from_ffi(affinity) {
        Ok(affinity) => affinity,
        Err(status) => return status,
    };
    let snapshot = session.session.snapshot();
    let source = match snapshot.byte_offset_for_utf16(Utf16Offset::new(source_utf16)) {
        Ok(source) => source,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let projection = match session.session.visual_text_for_visual_state() {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let bias = projection_bias_from_affinity(affinity);
    let visual = match projection.source_to_visual(source, bias) {
        Ok(visual) => visual,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let projected = projected_utf8(&projection);
    let visual_utf16 = match visual_utf16_offset(&projected, visual) {
        Ok(visual) => visual,
        Err(status) => return status,
    };
    let round_trip_source = match projection.visual_to_source(visual, bias) {
        Ok(source) => source,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_source_utf16 = match snapshot.utf16_offset(round_trip_source) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    // SAFETY: `output` was checked above and belongs to the caller.
    unsafe {
        *output = YuStorageProjectionCaret {
            revision: session.session.revision().get(),
            source_utf16,
            visual_utf16,
            round_trip_source_utf16,
            affinity: match affinity {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        };
    }
    YU_STORAGE_OK
}

/// Maps a visual UTF-16 selection from a native TextKit mirror back to the
/// canonical source. Non-collapsed ranges use outer Before/After projection
/// edges so hidden syntax remains selected in the source document.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projection_source_selection(
    session: *mut YuStorageSession,
    expected_revision: u64,
    visual_start_utf16: u64,
    visual_end_utf16: u64,
    affinity: u8,
    output: *mut YuStorageProjectionSourceSelection,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageProjectionSourceSelection::default() };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let affinity = match caret_affinity_from_ffi(affinity) {
        Ok(affinity) => affinity,
        Err(status) => return status,
    };
    if visual_start_utf16 > visual_end_utf16 {
        return YU_STORAGE_INVALID_SELECTION;
    }
    let projection = match session.session.visual_text_for_visual_state() {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let projected = projected_utf8(&projection);
    let visual_start_byte = match utf16_byte_offset(&projected, visual_start_utf16) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    let visual_end_byte = match utf16_byte_offset(&projected, visual_end_utf16) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    let visual_start = VisualOffset::new(match u64::try_from(visual_start_byte) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    });
    let visual_end = VisualOffset::new(match u64::try_from(visual_end_byte) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    });
    let (start_bias, end_bias) = if visual_start_utf16 == visual_end_utf16 {
        let bias = projection_bias_from_affinity(affinity);
        (bias, bias)
    } else {
        (Bias::Before, Bias::After)
    };
    let source_start = match projection.visual_to_source(visual_start, start_bias) {
        Ok(source) => source,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let source_end = match projection.visual_to_source(visual_end, end_bias) {
        Ok(source) => source,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    if source_start > source_end {
        return YU_STORAGE_INVALID_SELECTION;
    }
    let round_trip_visual_start = match projection.source_to_visual(source_start, start_bias) {
        Ok(visual) => visual,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_visual_end = match projection.source_to_visual(source_end, end_bias) {
        Ok(visual) => visual,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let snapshot = session.session.snapshot();
    let source_start_utf16 = match snapshot.utf16_offset(source_start) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let source_end_utf16 = match snapshot.utf16_offset(source_end) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_visual_start_utf16 =
        match visual_utf16_offset(&projected, round_trip_visual_start) {
            Ok(offset) => offset,
            Err(status) => return status,
        };
    let round_trip_visual_end_utf16 = match visual_utf16_offset(&projected, round_trip_visual_end) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    // SAFETY: output was checked above and belongs to the caller.
    unsafe {
        *output = YuStorageProjectionSourceSelection {
            revision: session.session.revision().get(),
            visual_start_utf16,
            visual_end_utf16,
            source_start_utf16,
            source_end_utf16,
            round_trip_visual_start_utf16,
            round_trip_visual_end_utf16,
            affinity: affinity_to_ffi(affinity),
        };
    }
    YU_STORAGE_OK
}

#[cfg(target_os = "macos")]
fn macos_query_layout_snapshot(
    session: &mut YuStorageSession,
    query: impl Into<yu_editor::LayoutQuery>,
    shaper: &CoreTextShaper,
) -> Result<std::sync::Arc<yu_editor::LayoutSnapshot>, i32> {
    let (images, intrinsics) = session
        .macos_render_host
        .as_ref()
        .map(|state| {
            (
                state
                    .image_resources
                    .publications
                    .values()
                    .cloned()
                    .collect::<Vec<_>>(),
                state
                    .image_resources
                    .intrinsics
                    .values()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default();
    let resize = session
        .table_resize_override
        .filter(|resize| resize.revision() == session.session.revision());
    yu_workspace::prepare_layout_snapshot_with_resources(
        session.session.document_mut().editor_mut(),
        query,
        shaper,
        &images,
        &intrinsics,
        resize,
    )
    .map_err(status_from_editor_error)
}

/// Resolves a document-space point through the current shaped block
/// layout. The endpoint is Revision-bound and uses the same published
/// viewport metrics as the native surface; it never asks the Swift/AppKit
/// mirror to approximate glyph positions. The returned `x`/`y` are snapped
/// document-space caret coordinates, while source/visual offsets are mapped
/// through the full lossless projection.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projection_hit_test(
    session: *mut YuStorageSession,
    expected_revision: u64,
    point_x: f32,
    point_y: f32,
    size: f32,
    max_width: f32,
    output: *mut YuStorageProjectionHit,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageProjectionHit::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            point_x,
            point_y,
            size,
            max_width,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        if !point_x.is_finite()
            || !point_y.is_finite()
            || !size.is_finite()
            || size <= 0.0
            || !max_width.is_finite()
            || max_width <= 0.0
        {
            return YU_STORAGE_EDITOR_ERROR;
        }
        let (shaper, metrics, _layout_config) = match macos_query_text_layout(
            session,
            size,
            max_width,
            session.session.viewport_config().layout().theme(),
        ) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(
            session,
            max_width,
            metrics,
            session.session.viewport_config().layout().theme(),
        ) {
            return status;
        }

        let query_y = point_y.max(0.0);
        let viewport = ViewportSpan::new(query_y, metrics.line_height());
        let snapshot = match macos_query_layout_snapshot(session, viewport, &shaper) {
            Ok(snapshot) => snapshot,
            Err(status) => return status,
        };
        let (placed, hit) = match snapshot.hit_test(LayoutPoint::new(point_x, query_y)) {
            Ok(Some(hit)) => hit,
            _ => return YU_STORAGE_INVALID_SELECTION,
        };
        let block = placed.metadata();
        let projection = match session.session.visual_text_for_visual_state() {
            Ok(projection) => projection,
            Err(error) => return storage_status(error),
        };
        let visual = match projection.source_to_visual(hit.source(), hit.bias()) {
            Ok(visual) => visual,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let projected = projected_utf8(&projection);
        let visual_utf16 = match visual_utf16_offset(&projected, visual) {
            Ok(offset) => offset,
            Err(status) => return status,
        };
        let round_trip_source = match projection.visual_to_source(visual, hit.bias()) {
            Ok(offset) => offset,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let source = session.session.snapshot();
        let source_utf16 = match source.utf16_offset(hit.source()) {
            Ok(offset) => offset.get(),
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let round_trip_source_utf16 = match source.utf16_offset(round_trip_source) {
            Ok(offset) => offset.get(),
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let (image_source_start_utf16, image_source_end_utf16) =
            match image_utf16_range(&source, hit.image()) {
                Ok(range) => range,
                Err(status) => return status,
            };
        let (content_source_start_utf16, content_source_end_utf16) =
            match image_utf16_range(&source, hit.content_source()) {
                Ok(range) => range,
                Err(status) => return status,
            };
        let markdown = session.session.document().editor().markdown();
        let marker = (!markdown.source_mode())
            .then(|| markdown.toc_marker_in(placed.layout().source_range()))
            .flatten();
        let navigation_target = marker
            .filter(|marker| hit.content_source() == Some(*marker))
            .and_then(|marker| {
                session
                    .session
                    .document_mut()
                    .editor_mut()
                    .toc_projection()
                    .ok()?
                    .target_for_hit(marker, placed.layout(), hit)
            });
        let (navigation_target_start_utf16, navigation_target_end_utf16) =
            match image_utf16_range(&source, navigation_target) {
                Ok(range) => range,
                Err(status) => return status,
            };
        let point = hit.point();
        // 折回文档坐标要补回内容原点：local_y 进 hit_test 时减过它（代码块的
        // 内容在盒里从上内边距起排），这里不补，返回的 caret y 比画出来的
        // 光标高 5pt。
        let document_y = placed.document_point(point).y();
        if !point.x().is_finite() || !document_y.is_finite() {
            return YU_STORAGE_EDITOR_ERROR;
        }
        let line_base = (block.y() / metrics.line_height()).floor().max(0.0) as u64;
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe {
            *output = YuStorageProjectionHit {
                revision: session.session.revision().get(),
                source_utf16,
                visual_utf16,
                round_trip_source_utf16,
                image_source_start_utf16,
                image_source_end_utf16,
                content_source_start_utf16,
                content_source_end_utf16,
                navigation_target_start_utf16,
                navigation_target_end_utf16,
                line: line_base.saturating_add(hit.line() as u64),
                x: point.x(),
                y: document_y,
                affinity: affinity_to_ffi(match hit.bias() {
                    Bias::Before => CaretAffinity::Upstream,
                    Bias::After => CaretAffinity::Downstream,
                }),
            };
        }
        YU_STORAGE_OK
    }
}

/// Returns metadata for the active transient composition projection. The
/// canonical source Revision is guarded by `expected_revision`; the returned
/// generation must be supplied to later count/fill and caret queries.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_composition_projection(
    session: *mut YuStorageSession,
    expected_revision: u64,
    output: *mut YuStorageCompositionProjection,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageCompositionProjection::default() };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let (metadata, _) = match composition_projection_metadata(&mut session.session) {
        Ok(value) => value,
        Err(status) => return status,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Resolves the active marked-text caret through the immutable shaped
/// composition snapshot. Canonical source and Revision remain unchanged; the
/// expected generation guards the transient preedit. Caret geometry is in
/// document content coordinates, while visual UTF-16 ranges use the full
/// transient projected stream.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_composition_shaped_caret(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    source_utf16: u64,
    affinity: u8,
    size: f32,
    max_width: f32,
    output: *mut YuStorageCompositionShapedCaret,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageCompositionShapedCaret::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            expected_generation,
            source_utf16,
            affinity,
            size,
            max_width,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) =
            validate_composition(&session.session, expected_revision, expected_generation)
        {
            return status;
        }
        let affinity = match caret_affinity_from_ffi(affinity) {
            Ok(affinity) => affinity,
            Err(status) => return status,
        };
        if !size.is_finite() || size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
            return YU_STORAGE_EDITOR_ERROR;
        }
        let (shaper, metrics, _layout_config) = match macos_query_text_layout(
            session,
            size,
            max_width,
            session.session.viewport_config().layout().theme(),
        ) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(
            session,
            max_width,
            metrics,
            session.session.viewport_config().layout().theme(),
        ) {
            return status;
        }

        let snapshot = session.session.snapshot();
        if snapshot
            .byte_offset_for_utf16(Utf16Offset::new(source_utf16))
            .is_err()
        {
            return YU_STORAGE_INVALID_SELECTION;
        }
        let (replacement, selection_start_utf16, selection_end_utf16) = {
            let Some(overlay) = session.session.composition() else {
                return YU_STORAGE_NO_OVERLAY;
            };
            (
                overlay.replacement_range(),
                overlay.selection_utf16().start().get(),
                overlay.selection_utf16().end().get(),
            )
        };
        let Some(block_index) = session
            .session
            .document()
            .editor()
            .block_index_for_source(replacement.start())
        else {
            return YU_STORAGE_INVALID_SELECTION;
        };
        let full_projection = match composition_projection(&mut session.session) {
            Ok(projection) => projection,
            Err(status) => return status,
        };
        let geometry = match macos_query_layout_snapshot(session, replacement.start(), &shaper) {
            Ok(snapshot) => snapshot,
            Err(status) => return status,
        };
        let Some(placed) = geometry.block(block_index) else {
            return YU_STORAGE_INVALID_SELECTION;
        };
        let layout = placed.layout();
        if layout.lines().is_empty() {
            return YU_STORAGE_INVALID_SELECTION;
        }
        let (block_visual, block_bias) = match composition_active_visual_caret(
            layout.visual(),
            selection_start_utf16,
            selection_end_utf16,
        ) {
            Ok(caret) => caret,
            Err(status) => return status,
        };
        let caret = match layout.caret_for_visual(block_visual, block_bias) {
            Ok(caret) => caret,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let (active_visual, active_bias) = match composition_active_visual_caret(
            &full_projection,
            selection_start_utf16,
            selection_end_utf16,
        ) {
            Ok(caret) => caret,
            Err(status) => return status,
        };
        let full_projected = projected_utf8(&full_projection);
        let visual_utf16 = match visual_utf16_offset(&full_projected, active_visual) {
            Ok(offset) => offset,
            Err(status) => return status,
        };
        let round_trip_source = match full_projection.visual_to_source(active_visual, active_bias) {
            Ok(offset) => offset,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let round_trip_source_utf16 = match snapshot.utf16_offset(round_trip_source) {
            Ok(offset) => offset.get(),
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let (visual_selection_start_utf16, visual_selection_end_utf16) =
            match composition_visual_selection_utf16(&full_projection, &full_projected) {
                Ok(selection) => selection,
                Err(status) => return status,
            };
        let (visual_replacement_start_utf16, visual_replacement_end_utf16) =
            match composition_visual_replacement_utf16(&full_projection, &full_projected) {
                Ok(replacement) => replacement,
                Err(status) => return status,
            };
        let point = placed.document_point(caret.point());
        let line_height = layout.caret_line_height(caret);
        if !point.x().is_finite()
            || !point.y().is_finite()
            || !line_height.is_finite()
            || line_height <= 0.0
        {
            return YU_STORAGE_EDITOR_ERROR;
        }
        let block_index = match u64::try_from(block_index) {
            Ok(index) => index,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let line_index = match u64::try_from(caret.line()) {
            Ok(index) => index,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe {
            *output = YuStorageCompositionShapedCaret {
                revision: snapshot.revision().get(),
                generation: session.session.composition_generation(),
                source_utf16,
                block_index,
                visual_utf16,
                round_trip_source_utf16,
                line_index,
                caret_x: point.x(),
                caret_y: point.y(),
                caret_width: 0.0,
                caret_height: line_height,
                visual_selection_start_utf16,
                visual_selection_end_utf16,
                visual_replacement_start_utf16,
                visual_replacement_end_utf16,
                affinity: affinity_to_ffi(affinity),
            };
        }
        YU_STORAGE_OK
    }
}

#[cfg(target_os = "macos")]
fn macos_table_resize_hit_at_point(
    session: &mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    point_x: f32,
    point_y: f32,
    tolerance: f32,
) -> Result<(usize, TableResizeHit), i32> {
    validate_table_resize_revision(session, expected_revision)?;
    if !size.is_finite()
        || size <= 0.0
        || !max_width.is_finite()
        || max_width <= 0.0
        || !point_x.is_finite()
        || !point_y.is_finite()
        || !tolerance.is_finite()
        || tolerance < 0.0
    {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    let (shaper, metrics, _layout_config) = macos_query_text_layout(
        session,
        size,
        max_width,
        session.session.viewport_config().layout().theme(),
    )?;
    macos_publish_viewport_config(
        session,
        max_width,
        metrics,
        session.session.viewport_config().layout().theme(),
    )?;

    let query_y = point_y.max(0.0);
    let viewport = ViewportSpan::new(query_y, metrics.line_height());
    let snapshot = macos_query_layout_snapshot(session, viewport, &shaper)?;
    let (placed, _) = snapshot
        .hit_test(LayoutPoint::new(point_x, query_y))
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    let block = placed.metadata();
    let layout = placed.layout();
    let Some(table) = layout.table() else {
        return Err(YU_STORAGE_INVALID_SELECTION);
    };
    let local_y = placed.local_point(LayoutPoint::new(point_x, query_y)).y();
    let hit = table
        .resize_hit_test(LayoutPoint::new(point_x, local_y), tolerance)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    Ok((block.index(), hit))
}

/// Resolves a document-space point against task checkbox geometry from the
/// current persistent render-host publication. This function never
/// reparses Markdown, rebuilds layout or mutates the editor. A native caller
/// may pass the returned block to the existing `ToggleTask` command only while
/// the returned Revision is still current.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_task_checkbox_hit_test(
    session: *mut YuStorageSession,
    expected_revision: u64,
    point_x: f32,
    point_y: f32,
    output: *mut YuStorageTaskCheckboxHit,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageTaskCheckboxHit::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session, expected_revision, point_x, point_y);
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        if !point_x.is_finite() || !point_y.is_finite() {
            return YU_STORAGE_EDITOR_ERROR;
        }
        if session.session.composition().is_some() {
            return YU_STORAGE_INVALID_STATE;
        }

        let hit = {
            let state = match session.macos_render_host.as_ref() {
                Some(state) => state,
                None => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
            };
            let publication = match state.builder.last_publication() {
                Some(publication) => publication,
                None => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
            };
            if publication.revision().get() != expected_revision
                || state.host.frame_revision() != Some(publication.revision())
                || state.host.frame_serial() != Some(publication.serial())
            {
                return YU_STORAGE_STALE_REVISION;
            }
            match publication
                .frame()
                .scene()
                .task_checkbox_hit_test(publication.revision(), Point::new(point_x, point_y))
            {
                Ok(Some(hit)) => hit,
                Ok(None) => return YU_STORAGE_INVALID_SELECTION,
                Err(_) => return YU_STORAGE_STALE_REVISION,
            }
        };

        let source = session.session.snapshot();
        let marker_start_utf16 = match source.utf16_offset(hit.source().start()) {
            Ok(offset) => offset.get(),
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let marker_end_utf16 = match source.utf16_offset(hit.source().end()) {
            Ok(offset) => offset.get(),
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let block_index = match u64::try_from(hit.block_index()) {
            Ok(index) => index,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let bounds = hit.bounds();
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe {
            *output = YuStorageTaskCheckboxHit {
                revision: hit.revision().get(),
                block_index,
                marker_start_utf16,
                marker_end_utf16,
                x: bounds.x(),
                y: bounds.y(),
                width: bounds.width(),
                height: bounds.height(),
            };
        }
        YU_STORAGE_OK
    }
}

/// 用一个文档坐标点探测或开始一次表格分隔线拖动。
///
/// 探测（hover 用的只读命中测试）与开始拖动此前是两个 FFI，参数只差一个
/// `pointer_position`，其余完全相同——同一次命中测试的两种用途。合成一个带
/// action 的入口后，「探测不得改变状态」这条约束写在一处而不是靠两份实现各自
/// 保证。
///
/// - `PROBE`：只解析，不开手势，`pointer_position` 不被读取。
/// - `BEGIN`：解析并开手势，之后由 `table_resize_action` 推进。
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_at_point(
    session: *mut YuStorageSession,
    expected_revision: u64,
    action: u8,
    size: f32,
    max_width: f32,
    point_x: f32,
    point_y: f32,
    tolerance: f32,
    pointer_position: f32,
    output: *mut YuStorageTableResizeHit,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageTableResizeHit::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            action,
            size,
            max_width,
            point_x,
            point_y,
            tolerance,
            pointer_position,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        if !matches!(
            action,
            YU_STORAGE_TABLE_RESIZE_PROBE | YU_STORAGE_TABLE_RESIZE_BEGIN
        ) {
            return YU_STORAGE_INVALID_COMMAND;
        }
        if action == YU_STORAGE_TABLE_RESIZE_BEGIN && !pointer_position.is_finite() {
            return YU_STORAGE_INVALID_SELECTION;
        }
        let (block_index, hit) = match macos_table_resize_hit_at_point(
            session,
            expected_revision,
            size,
            max_width,
            point_x,
            point_y,
            tolerance,
        ) {
            Ok(value) => value,
            Err(status) => return status,
        };
        let metadata = if action == YU_STORAGE_TABLE_RESIZE_BEGIN {
            match begin_table_resize_session(session, block_index, hit, pointer_position) {
                Ok(value) => value,
                Err(status) => return status,
            }
        } else {
            let block_index = match u64::try_from(block_index) {
                Ok(value) => value,
                Err(_) => return YU_STORAGE_INVALID_SELECTION,
            };
            match table_resize_hit_metadata(session.session.revision().get(), block_index, hit) {
                Ok(value) => value,
                Err(status) => return status,
            }
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe { *output = metadata };
        YU_STORAGE_OK
    }
}

/// Read-only column hover against the submitted frame. Attached windows never
/// shape text here; stale/unavailable geometry reports no hit. Headless callers
/// retain the synchronous probe for protocol checks.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_hover(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    point_x: f32,
    point_y: f32,
    tolerance: f32,
    output: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    unsafe { *output = 0 };
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            point_x,
            point_y,
            tolerance,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        if !size.is_finite()
            || size <= 0.0
            || !max_width.is_finite()
            || max_width <= 0.0
            || !scroll_y.is_finite()
            || scroll_y < 0.0
            || !viewport_height.is_finite()
            || viewport_height <= 0.0
            || !point_x.is_finite()
            || !point_y.is_finite()
            || !tolerance.is_finite()
            || tolerance < 0.0
        {
            return YU_STORAGE_EDITOR_ERROR;
        }
        if session.session.document().editor().composition().is_some() {
            return YU_STORAGE_OK;
        }
        let hit = if session
            .macos_render_host
            .as_ref()
            .is_some_and(|state| state.surface.is_some())
        {
            if std::env::var_os("YU_RENDER_TIMING").is_some() {
                println!("yu-render-metric event=hover_frame_query");
            }
            macos_matching_submitted_frame(session, size, max_width, scroll_y, viewport_height)
                .is_some_and(|frame| {
                    frame
                        .scene()
                        .tables()
                        .iter()
                        .any(|table| table.column_resize_hover(point_x, point_y, tolerance))
                })
        } else {
            if std::env::var_os("YU_RENDER_TIMING").is_some() {
                println!("yu-render-metric event=hover_layout_fallback");
            }
            match macos_table_resize_hit_at_point(
                session,
                expected_revision,
                size,
                max_width,
                point_x,
                point_y,
                tolerance,
            ) {
                Ok((_, hit)) => matches!(hit.target(), TableResizeTarget::Column { .. }),
                Err(YU_STORAGE_INVALID_SELECTION) => false,
                Err(status) => return status,
            }
        };
        unsafe { *output = u8::from(hit) };
        YU_STORAGE_OK
    }
}

/// 推进一次表格分隔线拖动。
///
/// update / finish / cancel 此前是三个独立 FFI，参数与前置条件完全一致，只在
/// 「对 gesture 做什么」上不同。它们是同一个指针手势的三个阶段，属于不变量 I3
/// 允许的「输入事件」这一类——一个带 action 的入口就够了。
///
/// - `UPDATE`：把 `pointer_position` 送进手势，返回本帧要用的临时几何。
/// - `FINISH`：结束手势，将最终列宽确认为文档呈现状态。
///   不产生任何 Markdown transaction。
/// - `CANCEL`：结束手势并清除活动覆盖；已确认的文档列宽不受影响。
///
/// `pointer_position` 只在 `UPDATE` 下被读取。`output` 三种 action 都必须可写：
/// 失败路径先把它清零，不留半成品（不变量 I4）。
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_action(
    session: *mut YuStorageSession,
    expected_revision: u64,
    action: u8,
    pointer_position: f32,
    output: *mut YuStorageTableResizeCommit,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageTableResizeCommit::default() };
    if !matches!(
        action,
        YU_STORAGE_TABLE_RESIZE_UPDATE
            | YU_STORAGE_TABLE_RESIZE_FINISH
            | YU_STORAGE_TABLE_RESIZE_CANCEL
    ) {
        return YU_STORAGE_INVALID_COMMAND;
    }
    if let Err(status) = validate_table_resize_revision(session, expected_revision) {
        return status;
    }
    let revision = session.session.revision();

    if action == YU_STORAGE_TABLE_RESIZE_CANCEL {
        let Some(gesture) = session.table_resize_gesture.take() else {
            session.table_resize_override = None;
            return YU_STORAGE_OK;
        };
        session.table_resize_override = None;
        return gesture
            .cancel(revision)
            .map_or_else(table_resize_gesture_status, |_| YU_STORAGE_OK);
    }

    let commit = if action == YU_STORAGE_TABLE_RESIZE_UPDATE {
        let Some(gesture) = session.table_resize_gesture.as_mut() else {
            return YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE;
        };
        if let Err(error) = gesture.update(revision, pointer_position) {
            session.table_resize_gesture = None;
            session.table_resize_override = None;
            return table_resize_gesture_status(error);
        }
        gesture.preview()
    } else {
        let Some(gesture) = session.table_resize_gesture.take() else {
            return YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE;
        };
        match gesture.finish(revision) {
            Ok(commit) => commit,
            Err(error) => {
                session.table_resize_override = None;
                return table_resize_gesture_status(error);
            }
        }
    };
    let metadata = match table_resize_commit_metadata(commit) {
        Ok(metadata) => metadata,
        Err(status) => return status,
    };
    if action == YU_STORAGE_TABLE_RESIZE_FINISH
        && matches!(commit.target(), TableResizeTarget::Column { .. })
    {
        if let Err(status) = confirm_session_table_widths(session, commit) {
            return status;
        }
        session.table_resize_override = None;
    } else {
        session.table_resize_override = Some(commit);
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Capture final geometry in the document, separate from the active gesture.
fn confirm_session_table_widths(
    session: &mut YuStorageSession,
    commit: TableResizeCommit,
) -> Result<(), i32> {
    let config = session.session.viewport_config().layout();
    #[cfg(target_os = "macos")]
    let shaper = {
        let size = session
            .macos_render_host
            .as_ref()
            .map_or(16.0, |host| host.size);
        core_text_layout(size, config.max_width(), config.theme())?.0
    };
    let editor = session.session.document_mut().editor_mut();
    let published = editor
        .current_layout_snapshot()
        .filter(|snapshot| snapshot.revision() == commit.revision())
        .and_then(|snapshot| {
            snapshot
                .block(commit.block_index())
                .map(|block| block.layout().clone())
        });
    let mut layout = if let Some(layout) = published {
        layout
    } else {
        #[cfg(target_os = "macos")]
        {
            editor
                .block_layout_for_visual_state_with_shaper(commit.block_index(), config, &shaper)
                .map_err(status_from_editor_error)?
        }
        #[cfg(not(target_os = "macos"))]
        {
            editor
                .block_layout_for_visual_state(commit.block_index(), config)
                .map_err(status_from_editor_error)?
        }
    };
    #[cfg(target_os = "macos")]
    layout
        .apply_table_resize_with_shaper(commit, &shaper)
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    #[cfg(not(target_os = "macos"))]
    layout
        .apply_table_resize(commit)
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    editor
        .confirm_table_column_widths(
            commit.block_index(),
            layout.table().ok_or(YU_STORAGE_EDITOR_ERROR)?,
        )
        .map_err(status_from_editor_error)
}

/// Shared body for the block-scoped and source-scoped caret queries.
///
/// `block_index` of `None` means "resolve the owning block from the source
/// offset". Platforms do not parse Markdown and therefore cannot know which
/// block owns an offset (invariant I1); resolving it here also keeps the
/// lookup and the layout inside one Revision check instead of letting an
/// intermediate block index race a concurrent edit.
#[cfg(target_os = "macos")]
fn macos_shaped_caret(
    session: &mut YuStorageSession,
    expected_revision: u64,
    block_index: Option<usize>,
    source_utf16: u64,
    affinity: u8,
    size: f32,
    max_width: f32,
) -> Result<YuStorageBlockCaret, i32> {
    validate_revision(&session.session, expected_revision)?;
    let affinity = caret_affinity_from_ffi(affinity)?;
    let (shaper, metrics, _config) = macos_query_text_layout(
        session,
        size,
        max_width,
        session.session.viewport_config().layout().theme(),
    )?;
    macos_publish_viewport_config(
        session,
        max_width,
        metrics,
        session.session.viewport_config().layout().theme(),
    )?;
    let snapshot = session.session.snapshot();
    let Ok(offset) = snapshot.byte_offset_for_utf16(Utf16Offset::new(source_utf16)) else {
        return Err(YU_STORAGE_INVALID_SELECTION);
    };
    let block_index = match block_index {
        Some(index) if index < session.session.block_count() => index,
        Some(_) => return Err(YU_STORAGE_INVALID_SELECTION),
        None => session
            .session
            .document()
            .editor()
            .block_index_for_source(offset)
            .ok_or(YU_STORAGE_INVALID_SELECTION)?,
    };
    let snapshot = macos_query_layout_snapshot(session, offset, &shaper)?;
    let placed = snapshot
        .block(block_index)
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    let layout = placed.layout();
    let mut metadata = block_caret_from_layout(
        &session.session,
        block_index,
        source_utf16,
        affinity,
        layout,
        metrics.line_height(),
        1,
    )?;
    metadata.caret_y += placed.content_y();
    Ok(metadata)
}

/// Resolves a source caret's shaped geometry without the caller naming a block.
///
/// The platform needs this for IME candidate-window placement: AppKit's
/// `firstRect(forCharacterRange:)` must report where the caret actually is on
/// screen, and only the Rust layout knows that — TextKit lays out canonical
/// source while the screen shows the projection (invariants H3, I1).
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_source_caret(
    session: *mut YuStorageSession,
    expected_revision: u64,
    source_utf16: u64,
    affinity: u8,
    size: f32,
    max_width: f32,
    output: *mut YuStorageBlockCaret,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageBlockCaret::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            source_utf16,
            affinity,
            size,
            max_width,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    match macos_shaped_caret(
        session,
        expected_revision,
        None,
        source_utf16,
        affinity,
        size,
        max_width,
    ) {
        // SAFETY: output was checked for null and belongs to the caller.
        Ok(caret) => unsafe {
            *output = caret;
            YU_STORAGE_OK
        },
        Err(status) => status,
    }
}

/// Returns Revision-bound, document-space metadata for visible table column
/// dividers. The count/fill contract is read-only and intentionally separate
/// from the resize gesture ABI so Accessibility can enumerate targets without
/// opening a session or retaining a Rust layout object. An existing
/// session-only column override is reflected in the returned geometry.
///
/// # Safety
/// `session` must be live; `written` must be writable; `dividers` must point to
/// `capacity` writable values when `capacity > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_accessibility_dividers(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    dividers: *mut YuStorageTableResizeAccessibilityDivider,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && dividers.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` was checked for null and belongs to the caller.
    unsafe { *written = 0 };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            dividers,
            capacity,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        if !size.is_finite()
            || size <= 0.0
            || !max_width.is_finite()
            || max_width <= 0.0
            || !scroll_y.is_finite()
            || scroll_y < 0.0
            || !viewport_height.is_finite()
            || viewport_height <= 0.0
        {
            return YU_STORAGE_EDITOR_ERROR;
        }
        if let Some(result) =
            macos_published_table_dividers(session, size, max_width, scroll_y, viewport_height)
        {
            let encoded = match result {
                Ok(value) => value,
                Err(status) => return status,
            };
            unsafe { *written = encoded.len() };
            if capacity == 0 && dividers.is_null() {
                return YU_STORAGE_OK;
            }
            if encoded.len() > capacity {
                return YU_STORAGE_BUFFER_TOO_SMALL;
            }
            if !encoded.is_empty() {
                unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), dividers, encoded.len()) };
            }
            return YU_STORAGE_OK;
        }
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!("yu-render-metric event=ax_layout_fallback");
        }
        let (shaper, metrics, _layout_config) = match macos_query_text_layout(
            session,
            size,
            max_width,
            session.session.viewport_config().layout().theme(),
        ) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(
            session,
            max_width,
            metrics,
            session.session.viewport_config().layout().theme(),
        ) {
            return status;
        }

        let viewport = ViewportSpan::new(scroll_y, viewport_height);
        if session.session.composition().is_some() {
            return YU_STORAGE_OK;
        }
        let geometry = match macos_query_layout_snapshot(session, viewport, &shaper) {
            Ok(snapshot) => snapshot,
            Err(status) => return status,
        };
        let source = geometry.source();
        let divider_width = (metrics.default_advance() * 0.25).max(1.0);
        let mut encoded = Vec::new();
        for placed in geometry.blocks() {
            let block = placed.metadata();
            let layout = placed.layout();
            let Some(table) = layout.table() else {
                continue;
            };
            let block_index = match u64::try_from(block.index()) {
                Ok(index) => index,
                Err(_) => return YU_STORAGE_INVALID_SELECTION,
            };
            let metadata = match table_resize_accessibility_metadata(
                source,
                geometry.revision().get(),
                block_index,
                placed.content_y(),
                divider_width,
                table,
            ) {
                Ok(metadata) => metadata,
                Err(status) => return status,
            };
            encoded.extend(metadata);
        }

        // SAFETY: `written` was checked for null and belongs to the caller.
        unsafe { *written = encoded.len() };
        if capacity == 0 && dividers.is_null() {
            return YU_STORAGE_OK;
        }
        if encoded.len() > capacity {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded.is_empty() {
            // SAFETY: capacity was checked against the encoded count.
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), dividers, encoded.len()) };
        }
        YU_STORAGE_OK
    }
}

#[cfg(target_os = "macos")]
fn image_utf16_range(source: &TextSnapshot, range: Option<TextRange>) -> Result<(u64, u64), i32> {
    let Some(range) = range else {
        return Ok((
            YU_STORAGE_IMAGE_DESTINATION_NONE,
            YU_STORAGE_IMAGE_DESTINATION_NONE,
        ));
    };
    let start = source
        .utf16_offset(range.start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let end = source
        .utf16_offset(range.end())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    Ok((start, end))
}

/// 一个块上的全部图片。
#[cfg(target_os = "macos")]
fn block_images(decorations: &yu_editor::BlockDecorations) -> Vec<ImageSpan> {
    decorations
        .widgets()
        .iter()
        .copied()
        .filter_map(|widget| match widget {
            yu_editor::BlockWidget::Image(image) => Some(image),
            yu_editor::BlockWidget::Checkbox(_) | yu_editor::BlockWidget::Embedded(_) => None,
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn image_resource_key(
    source: &TextSnapshot,
    image: ImageSpan,
    definitions: &yu_markdown::ReferenceDefinitionIndex,
) -> Option<ImageKey> {
    ImageKey::new(yu_markdown::image_destination_text(
        source,
        image,
        definitions,
    )?)
    .ok()
}

#[cfg(target_os = "macos")]
fn macos_image_resource_status(
    host: Option<&MacosRenderHostState>,
    key: Option<&ImageKey>,
    revision: u64,
) -> u8 {
    let (Some(host), Some(key)) = (host, key) else {
        return YU_STORAGE_IMAGE_RESOURCE_UNKNOWN;
    };
    let resources = &host.image_resources;
    let fingerprint = key.fingerprint();
    if resources.publications.contains_key(&fingerprint) {
        return YU_STORAGE_IMAGE_RESOURCE_READY;
    }
    if resources.in_flight.contains(key) {
        return YU_STORAGE_IMAGE_RESOURCE_PENDING;
    }
    if resources
        .cache
        .failure(key)
        .is_some_and(|failure| failure.revision().get() == revision)
    {
        return YU_STORAGE_IMAGE_RESOURCE_FAILED;
    }
    if resources.intrinsics.contains_key(&fingerprint) {
        return YU_STORAGE_IMAGE_RESOURCE_PENDING;
    }
    YU_STORAGE_IMAGE_RESOURCE_UNKNOWN
}

#[cfg(target_os = "macos")]
fn macos_image_requests(
    session: &mut YuStorageSession,
    block_indices: &[(usize, ImageRequestPriority)],
) -> Result<ImageRequestPlan, i32> {
    let source = session.session.snapshot();
    let revision = session.session.revision();
    let definitions = session
        .session
        .document()
        .editor()
        .markdown()
        .reference_definitions()
        .clone();
    let mut candidates = Vec::new();
    for &(block_index, priority) in block_indices {
        let decorations = session
            .session
            .block_decorations(block_index)
            .map_err(storage_status)?;
        for image in block_images(&decorations) {
            let Some(key) = image_resource_key(&source, image, &definitions) else {
                continue;
            };
            let request = ImageRequest::new(revision, image.source(), key.destination().to_owned())
                .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
            candidates.push(ImageRequestCandidate::new(request, block_index, priority));
        }
    }
    Ok(ImageRequestPlan::from_candidates(candidates))
}

#[cfg(target_os = "macos")]
fn macos_embedded_resource_status(result: EmbeddedRequestResult) -> u8 {
    match result {
        EmbeddedRequestResult::Ready(_) => YU_STORAGE_EMBEDDED_RESOURCE_READY,
        EmbeddedRequestResult::Pending => YU_STORAGE_EMBEDDED_RESOURCE_PENDING,
        EmbeddedRequestResult::Failed(failure) => match failure.kind() {
            EmbeddedFailureKind::Unsupported => YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED,
            EmbeddedFailureKind::InvalidSource
            | EmbeddedFailureKind::Render
            | EmbeddedFailureKind::Worker => YU_STORAGE_EMBEDDED_RESOURCE_FAILED,
        },
    }
}

#[cfg(target_os = "macos")]
fn macos_render_host_error_status(error: &CoreTextViewportFrameError) -> i32 {
    eprintln!("yu-native-frame: {error}");
    match error {
        CoreTextViewportFrameError::Cancelled => {
            if std::env::var_os("YU_RENDER_TIMING").is_some() {
                println!("yu-render-metric event=preparation_cancelled");
            }
            YU_STORAGE_RENDER_BUSY
        }
        CoreTextViewportFrameError::InvalidConfig(_) => YU_STORAGE_INVALID_VIEWPORT_CONFIG,
        CoreTextViewportFrameError::Raster(_) => YU_STORAGE_SHAPER_UNAVAILABLE,
        CoreTextViewportFrameError::Document(error) => status_from_editor_error(error.clone()),
        CoreTextViewportFrameError::Atlas(_) | CoreTextViewportFrameError::Publish(_) => {
            YU_STORAGE_RENDER_HOST_UNAVAILABLE
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_render_host_config(
    viewport: ViewportSpan,
    size: f32,
    max_width: f32,
    viewport_height: f32,
    raster_scale: f32,
    appearance: Appearance,
) -> Result<ViewportRenderConfig, i32> {
    let scene_height = viewport_height.max(1.0);
    let scene_viewport = Rect::new(0.0, viewport.scroll_y(), max_width, scene_height)
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    // **这里一个颜色字面量都没有了。** 平台送进来的是「现在是深还是浅」这个
    // 事实，颜色由 `yu-workspace` 挑——「产品选色住在那一层」这条以前在三处
    // 文档里写着，而背景与这几个装饰色是它唯一的例外。第二端因此不必把同一套
    // 配色再挑一遍（挑出来的一定会漂开）。
    Ok(
        ViewportRenderConfig::new(viewport, size, scene_viewport, appearance.text())
            .with_raster_scale(raster_scale)
            // Rust surface 是唯一渲染路径，背景必须由这一帧自己画出来
            // （不变量 I5）：Metal layer 是透明的，未触及的像素会露出下层视图，
            // 而下层视图已经不再绘制任何东西。
            .with_background(appearance.background())
            .with_editor_decorations(appearance.editor_decorations())
            .with_appearance(appearance),
    )
}

#[cfg(target_os = "macos")]
fn macos_editor_decoration_counts(scene: &yu_scene::Scene) -> Result<(u64, u64, u64), i32> {
    let mut selection = 0_u64;
    let mut caret = 0_u64;
    let mut search = 0_u64;
    for primitive in scene.primitives() {
        let Primitive::EditorDecoration(decoration) = primitive else {
            continue;
        };
        let counter = match decoration.role() {
            EditorDecorationPrimitiveRole::Selection => &mut selection,
            EditorDecorationPrimitiveRole::Caret
            | EditorDecorationPrimitiveRole::CompositionCaret => &mut caret,
            EditorDecorationPrimitiveRole::SearchMatch
            | EditorDecorationPrimitiveRole::SearchCurrent => &mut search,
        };
        *counter = counter
            .checked_add(1)
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    }
    Ok((selection, caret, search))
}

/// 这一帧有几个字形的颜色不是正文色。
///
/// 「正文色」取这一帧渲染配置里的那个值，不是一个写死的常数：配置换了颜色而
/// 这里没跟着换，表现会是「整篇文档都被算成高亮」——一个不报错的假绿。
#[cfg(target_os = "macos")]
fn macos_highlighted_glyph_count(
    scene: &yu_scene::Scene,
    text_color: yu_scene::Rgba8,
) -> Result<u64, i32> {
    let mut count = 0_u64;
    for primitive in scene.primitives() {
        let Primitive::Glyph(glyph) = primitive else {
            continue;
        };
        if glyph.color() == text_color {
            continue;
        }
        count = count
            .checked_add(1)
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    }
    Ok(count)
}

#[cfg(target_os = "macos")]
fn macos_render_host_snapshot(
    state: &MacosRenderHostState,
    composition_generation: u64,
) -> Result<YuStorageMacosRenderHostSnapshot, i32> {
    let publication = state
        .last_publication
        .as_ref()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let frame = publication.frame();
    let plan = frame.plan();
    let config = state.builder.config();
    let input = frame.scene().input();
    let frame_revision = state
        .host
        .frame_revision()
        .map_or(u64::MAX, |revision| revision.get());
    let frame_serial = state.host.frame_serial().unwrap_or(u64::MAX);
    let (selection_decoration_count, caret_decoration_count, search_decoration_count) =
        macos_editor_decoration_counts(frame.scene().scene())?;
    let highlighted_glyph_count =
        macos_highlighted_glyph_count(frame.scene().scene(), config.color())?;
    Ok(YuStorageMacosRenderHostSnapshot {
        revision: publication.revision().get(),
        composition_generation,
        frame_revision,
        surface_generation: state.host.surface_generation(),
        frame_serial,
        command_count: u64::try_from(plan.commands().len())
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
        upload_count: u64::try_from(plan.uploads().len())
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
        damage_count: u64::try_from(plan.damage().len())
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
        atlas_page_count: u64::try_from(state.builder.atlas_page_count())
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
        atlas_glyph_count: u64::try_from(state.builder.atlas_glyph_count())
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
        atlas_bytes: u64::try_from(state.builder.atlas_bytes())
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
        content_height: input.content_height(),
        scroll_y: config.viewport().scroll_y(),
        viewport_height: config.viewport().height(),
        max_scroll_y: (input.content_height() - config.viewport().height()).max(0.0),
        viewport_width: config.scene_viewport().width(),
        published: 1,
        selection_decoration_count,
        caret_decoration_count,
        search_decoration_count,
        highlighted_glyph_count,
        // 调用方在离开 host 借用之后填入；这里没有 session 可查。
        resource_refresh_pending: u8::from(state.resource_refresh_pending),
        resource_retry_pending: u8::from(state.resource_retry_pending),
        layout_pending: u8::from(state.layout_pending),
    })
}

#[cfg(target_os = "macos")]
/// 一次 retained frame 请求的全部参数。
///
/// 打包而非平铺：这些值总是同进同出，且必须来自同一次平台查询——把它们拆成
/// 独立参数容易在调用点混入不同来源的值（例如上一帧的 scroll 配这一帧的
/// scale），而那类错误编译器发现不了。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
struct MacosFrameRequest {
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    surface_width: f64,
    surface_height: f64,
    surface_generation: u64,
    /// backing scale：字形按它取样，后端再除回逻辑坐标。
    raster_scale: f32,
    /// 系统外观。**只有平台知道**（AppKit 的 `effectiveAppearance`），
    /// 而选出哪一套颜色是 `yu-workspace` 的事——进来的是一个事实，
    /// 出去的是一整套颜色。
    appearance: Appearance,
}

#[cfg(target_os = "macos")]
fn macos_render_host_frame(
    session: &mut YuStorageSession,
    request: MacosFrameRequest,
    allow_background: bool,
) -> Result<YuStorageMacosRenderHostSnapshot, i32> {
    let capture_start = std::time::Instant::now();
    // Capture before draining workers; a completion racing this build must
    // remain dirty so the following presentation cannot skip its publication.
    let resource_generation = yu_render_macos::resource_completion_generation();
    let MacosFrameRequest {
        expected_revision,
        size,
        max_width,
        scroll_y,
        viewport_height,
        surface_width,
        surface_height,
        surface_generation,
        raster_scale,
        appearance,
    } = request;
    validate_revision(&session.session, expected_revision)?;
    let color = appearance.theme().text();
    let foreground = u32::from_be_bytes([color.red(), color.green(), color.blue(), color.alpha()]);
    let style = yu_assets::EmbeddedStyle::new(
        size,
        foreground,
        matches!(appearance, Appearance::Dark | Appearance::YuDark),
    )
    .ok_or(YU_STORAGE_EDITOR_ERROR)?
    .with_reference_day(session.macos_embedded_resources.style.reference_day());
    session
        .macos_embedded_resources
        .set_style(style, appearance.theme_id());

    session
        .macos_embedded_resources
        .advance(session.session.snapshot().revision())?;
    let resources = session.macos_embedded_resources.publications.clone();
    let failed_ranges = session
        .macos_embedded_resources
        .diagnostics
        .iter()
        .map(|(_, range, _)| *range)
        .collect();
    let editor = session.session.document_mut().editor_mut();
    let sizes = resources
        .iter()
        .filter_map(|publication| {
            let dimensions = publication.payload().dimensions();
            yu_editor::ImageIntrinsicSize::new(dimensions.width(), dimensions.height())
                .and_then(|size| size.with_baseline(publication.payload().baseline_milli()))
                .ok()
                .map(|size| (publication.source_range(), size))
        })
        .collect();
    let resource_revision = editor.revision();
    editor
        .set_failed_resource_ranges(resource_revision, failed_ranges)
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    editor
        .set_embedded_sizes(resource_revision, sizes)
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;

    if !size.is_finite()
        || size <= 0.0
        || !max_width.is_finite()
        || max_width <= 0.0
        || !scroll_y.is_finite()
        || scroll_y < 0.0
        || !viewport_height.is_finite()
        || viewport_height <= 0.0
    {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    let rebuild = session.macos_render_host.as_ref().is_none_or(|state| {
        (state.size - size).abs() > 0.001
            || state.builder.config().raster_scale() != raster_scale
            || state.builder.config().appearance() != appearance
    });
    let (metrics, shaper) = if rebuild {
        let (shaper, metrics, _layout_config) =
            macos_query_text_layout(session, size, max_width, appearance.theme_id())?;
        (metrics, Some(shaper))
    } else {
        let state = session
            .macos_render_host
            .as_ref()
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        (state.metrics, None)
    };
    macos_publish_viewport_config(session, max_width, metrics, appearance.theme_id())?;
    // Keep at least one viewport of already-shaped content around the active
    // camera.  Scroll-only presentations can then reuse the retained frame
    // instead of synchronously rebuilding every block for each wheel sample.
    let published_viewport = session.session.viewport_config();
    if published_viewport.overscan() < viewport_height {
        session
            .session
            .document_mut()
            .editor_mut()
            .set_viewport_overscan(viewport_height)
            .map_err(|_| YU_STORAGE_INVALID_VIEWPORT_CONFIG)?;
    }

    let revision = session.session.revision();
    let table_resize = match session.table_resize_override {
        Some(commit) if commit.revision() == revision => {
            if matches!(commit.target(), TableResizeTarget::Column { .. }) {
                Some(commit)
            } else {
                None
            }
        }
        Some(_) => {
            session.table_resize_override = None;
            None
        }
        None => None,
    };
    let viewport = ViewportSpan::new(scroll_y, viewport_height);
    let config = macos_render_host_config(
        viewport,
        size,
        max_width,
        viewport_height,
        raster_scale,
        appearance,
    )?;
    let config = config.with_editor_decorations(
        appearance
            .editor_decorations()
            .with_focus_mode(session.focus_mode),
    );
    let config = table_resize.map_or(config, |commit| config.with_table_resize(commit));
    if session
        .macos_render_host
        .as_ref()
        .is_some_and(|state| surface_generation < state.host.surface_generation())
    {
        return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE);
    }
    if rebuild {
        let initial_serial = session
            .macos_render_host
            .as_ref()
            .and_then(|state| state.last_publication.as_ref())
            .map_or(0, |publication| publication.serial());
        let shaper = shaper.ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;

        let builder = CoreTextViewportFrameBuilder::with_shaper_and_initial_serial(
            shaper,
            config,
            GlyphAtlasConfig::default(),
            initial_serial,
        )
        .map_err(|error| macos_render_host_error_status(&error))?;
        let previous = session.macos_render_host.take();
        let (surface, image_resources, worker, sequence, binding, last_publication) = match previous
        {
            Some(state) => {
                if let Some(worker) = state.frame_worker.as_ref() {
                    worker.invalidate();
                }
                (
                    state.surface,
                    state.image_resources,
                    state.frame_worker,
                    state.frame_request_generation,
                    state.binding_generation,
                    state.last_publication,
                )
            }
            None => (
                None,
                MacosImageResourceState::new()?,
                macos_new_frame_worker(0),
                0,
                0,
                None,
            ),
        };
        session.macos_render_host = Some(MacosRenderHostState {
            builder,
            host: MetalViewportHostSession::new(revision, surface_generation),
            last_publication,
            size,
            surface,
            image_resources,
            // 重建 host 后没有可复用的帧，下一次提交必须真正执行。
            last_frame_key: None,
            prepared_build: None,
            resource_refresh_pending: false,
            resource_retry_pending: false,
            layout_pending: false,
            resource_completion_generation: resource_generation,
            frame_worker: worker,
            frame_worker_request: None,
            frame_request_generation: sequence,
            binding_generation: binding,
            visible_blocks: Vec::new(),
            visibility_revision: None,
            metrics,
        });
    }

    {
        let state = session
            .macos_render_host
            .as_mut()
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        state
            .builder
            .update_config(config)
            .map_err(|error| macos_render_host_error_status(&error))?;
    }
    if allow_background {
        return macos_render_host_background_frame(session, request, config, capture_start);
    }
    if let Some(state) = session.macos_render_host.as_mut() {
        if let Some(worker) = state.frame_worker.as_ref() {
            worker.invalidate();
        }
        state.frame_worker_request = None;
        state.builder.ensure_publication_serial(
            state
                .last_publication
                .as_ref()
                .map_or(0, |publication| publication.serial()),
        );
    }
    let document_path = session.session.path().to_path_buf();
    let layout_timing_start = std::time::Instant::now();
    let viewport_blocks = {
        let state = session
            .macos_render_host
            .as_mut()
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        let document = session.session.document_mut().editor_mut();
        state
            .builder
            .viewport_image_blocks(document)
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?
    };
    let layout_elapsed = layout_timing_start.elapsed();
    let image_requests = macos_image_requests(session, &viewport_blocks)?;
    let requested_build = frame_key(
        session,
        appearance,
        FrameGeometry::new(
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_width,
            surface_height,
            f64::from(raster_scale),
        )
        .ok_or(YU_STORAGE_EDITOR_ERROR)?,
    )
    .build()
    .clone();
    let state = session
        .macos_render_host
        .as_mut()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state
        .host
        .advance_revision(revision)
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state
        .host
        .sync_surface_generation(surface_generation)
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state.image_resources.sync(
        image_requests,
        revision,
        document_path,
        (surface_width as f32 * raster_scale).ceil() as u32,
    )?;
    let image_publications = state
        .image_resources
        .publications
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let image_intrinsics = state
        .image_resources
        .intrinsics
        .values()
        .cloned()
        .collect::<Vec<_>>();
    state
        .builder
        .set_embedded_publications(session.macos_embedded_resources.publications.clone());
    let publish_timing_start = std::time::Instant::now();
    let publication = {
        let document = session.session.document_mut().editor_mut();
        document.set_resource_geometry_version(state.image_resources.geometry_version(revision));
        state
            .builder
            .publish_with_images_and_intrinsics(document, &image_publications, &image_intrinsics)
            .map_err(|error| macos_render_host_error_status(&error))?
    };
    let publish_elapsed = publish_timing_start.elapsed();
    state
        .host
        .accept_publication(publication.clone())
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state.last_publication = Some(publication);
    state.prepared_build = Some(requested_build);
    let mut snapshot = macos_render_host_snapshot(state, session.session.composition_generation())?;
    // 在离开 host 的可变借用之后再问：这一帧的可见资源是否已经全部落定。
    let visible_blocks = viewport_blocks
        .iter()
        .map(|&(index, _)| index)
        .collect::<Vec<_>>();
    let (pending, retry) = macos_frame_needs_resource_refresh(session, &visible_blocks)?;
    snapshot.resource_refresh_pending = u8::from(pending);
    snapshot.resource_retry_pending = u8::from(retry);
    if let Some(state) = session.macos_render_host.as_mut() {
        state.resource_refresh_pending = snapshot.resource_refresh_pending != 0;
        state.resource_retry_pending = retry;
        state.resource_completion_generation = resource_generation;
    }
    if std::env::var_os("YU_RENDER_TIMING").is_some() {
        eprintln!(
            "yu-render phases layout={:?} publish={:?}",
            layout_elapsed, publish_elapsed
        );
    }
    Ok(snapshot)
}

#[cfg(target_os = "macos")]
fn macos_render_host_background_frame(
    session: &mut YuStorageSession,
    request: MacosFrameRequest,
    config: ViewportRenderConfig,
    capture_start: std::time::Instant,
) -> Result<YuStorageMacosRenderHostSnapshot, i32> {
    let revision = session.session.revision();
    let resources = yu_render_macos::resource_completion_generation();
    let geometry = FrameGeometry::new(
        request.size,
        request.max_width,
        request.scroll_y,
        request.viewport_height,
        request.surface_width,
        request.surface_height,
        f64::from(request.raster_scale),
    )
    .ok_or(YU_STORAGE_EDITOR_ERROR)?;
    let key = frame_key(session, request.appearance, geometry)
        .build()
        .clone();
    let state = session
        .macos_render_host
        .as_mut()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let pending_matches = state.frame_worker_request.as_ref().is_some_and(|ticket| {
        ticket.matches_input(
            &key,
            request.surface_generation,
            state.binding_generation,
            resources,
        )
    });

    let completed = if pending_matches {
        // This fast path does not clone a document, parse, probe layout, or
        // poll resource caches while the worker is still preparing the frame.
        let worker = state
            .frame_worker
            .as_ref()
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        let Some((generation, result)) = worker
            .try_take()
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?
        else {
            return Err(YU_STORAGE_RENDER_BUSY);
        };
        let current = state
            .frame_worker_request
            .as_ref()
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        if generation != state.frame_request_generation || !result.ticket.accepts(current) {
            state.frame_worker_request = None;
            return Err(YU_STORAGE_RENDER_BUSY);
        }
        result
    } else {
        if let Some(worker) = state.frame_worker.as_ref() {
            worker.invalidate();
        }
        state.frame_worker_request = None;
        let visible = if state.visibility_revision == Some(revision) {
            state.visible_blocks.clone()
        } else {
            Vec::new()
        };
        let plan = macos_image_requests(session, &visible)?;
        let document_path = session.session.path().to_path_buf();
        let state = session
            .macos_render_host
            .as_mut()
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        state.image_resources.sync(
            plan,
            revision,
            document_path,
            (state.builder.config().scene_viewport().width()
                * state.builder.config().raster_scale())
            .ceil() as u32,
        )?;
        let editor = session.session.document_mut().editor_mut();
        editor.set_resource_geometry_version(state.image_resources.geometry_version(revision));
        let document = editor.capture_render_snapshot();
        state.frame_request_generation = state
            .frame_request_generation
            .checked_add(1)
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        let ticket = MacosFrameTicket {
            request: FrameBuildRequest::new(key.clone(), state.frame_request_generation),
            surface_generation: request.surface_generation,
            binding_generation: state.binding_generation,
            resource_generation: resources,
        };
        let input = ViewportFrameBuildInput {
            request: ticket.request.clone(),
            document,
            image_publications: state
                .image_resources
                .publications
                .values()
                .cloned()
                .collect(),
            image_intrinsics: state.image_resources.intrinsics.values().cloned().collect(),
            embedded_publications: session.macos_embedded_resources.publications.clone(),
        };
        state.frame_worker_request = Some(ticket.clone());
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!(
                "yu-render-metric event=input_capture duration_ms={:.6}",
                capture_start.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some(worker) = state.frame_worker.as_ref() {
            worker
                .submit(
                    ticket.request.generation(),
                    MacosFrameBuildJob {
                        input,
                        shaper: state.builder.shaper().clone(),
                        config,
                        ticket,
                        minimum_serial: state
                            .last_publication
                            .as_ref()
                            .map_or(0, |publication| publication.serial()),
                    },
                )
                .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
            return Err(YU_STORAGE_RENDER_BUSY);
        }
        // Preserve the existing degraded recovery when the OS refuses to spawn
        // a worker. Normal operation never reconstructs a document here.
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!("yu-render-metric event=worker_fallback");
        }
        state.builder.ensure_publication_serial(
            state
                .last_publication
                .as_ref()
                .map_or(0, |publication| publication.serial()),
        );
        MacosFrameBuildResult {
            ticket,
            output: state
                .builder
                .publish_owned(input)
                .map_err(|error| macos_render_host_error_status(&error)),
        }
    };

    let ticket = completed.ticket;
    let output = match completed.output {
        Ok(output) => output,
        Err(status) => {
            if let Some(state) = session.macos_render_host.as_mut() {
                state.frame_worker_request = None;
            }
            return Err(status);
        }
    };
    // Validate the entire ticket before collecting metadata or touching the
    // accepted frame/CPU atlas/GPU state. Revision is also checked explicitly.
    let state = session
        .macos_render_host
        .as_mut()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let current = state
        .frame_worker_request
        .as_ref()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    if !ticket.accepts(current)
        || !session
            .session
            .document()
            .editor()
            .accepts_layout_snapshot(&output.layout)
        || ticket.request.generation() != state.frame_request_generation
        || output.request != ticket.request
        || output.publication.revision() != revision
        || !ticket.matches_input(
            &key,
            request.surface_generation,
            state.binding_generation,
            yu_render_macos::resource_completion_generation(),
        )
        || !(output.publication.frame().plan().viewport().y() == request.scroll_y
            || macos_frame_covers_viewport(
                output.publication.frame(),
                request.scroll_y,
                request.viewport_height,
            ))
    {
        state.frame_worker_request = None;
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!("yu-render-metric event=stale_publication");
        }
        return Err(YU_STORAGE_RENDER_BUSY);
    }
    state.frame_worker_request = None;
    if !session
        .session
        .document_mut()
        .editor_mut()
        .integrate_layout_measurements(&output.layout)
    {
        return Err(YU_STORAGE_RENDER_BUSY);
    }
    // Visible block discovery was done by the worker. Keep this small result
    // so subsequent resource notifications can be collected without layout.
    state.visible_blocks = output.viewport_blocks.clone();
    state.visibility_revision = Some(revision);
    let plan = macos_image_requests(session, &output.viewport_blocks)?;
    let path = session.session.path().to_path_buf();
    let state = session
        .macos_render_host
        .as_mut()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state.image_resources.sync(
        plan,
        revision,
        path,
        (state.builder.config().scene_viewport().width() * state.builder.config().raster_scale())
            .ceil() as u32,
    )?;
    let images = state
        .image_resources
        .publications
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let intrinsics = state
        .image_resources
        .intrinsics
        .values()
        .cloned()
        .collect::<Vec<_>>();
    // A newly visible cache hit can change resource inputs without a worker
    // completion notification. Never pair its GPU pixels with old geometry.
    if output.image_publications != images
        || output.image_intrinsics != intrinsics
        || ticket.resource_generation != yu_render_macos::resource_completion_generation()
    {
        if std::env::var_os("YU_RENDER_TIMING").is_some() {
            println!("yu-render-metric event=stale_publication");
        }
        return Err(YU_STORAGE_RENDER_BUSY);
    }
    state
        .host
        .advance_revision(revision)
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state
        .host
        .sync_surface_generation(request.surface_generation)
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state
        .host
        .accept_publication(output.publication.clone())
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state.layout_pending = output.layout.layout_pending();
    state.last_publication = Some(output.publication);
    state.prepared_build = Some(key);
    state.builder.replace_atlas(output.atlas);
    state.resource_completion_generation = ticket.resource_generation;
    // Install the measurements used by the accepted frame before AppKit asks
    // for caret/hit-test geometry. Rejected worker outputs never reach here.
    let adopted = session
        .session
        .document_mut()
        .editor_mut()
        .adopt_layout_snapshot(output.layout);
    debug_assert!(
        adopted,
        "layout was validated before publication acceptance"
    );
    let visible = output
        .viewport_blocks
        .iter()
        .map(|&(index, _)| index)
        .collect::<Vec<_>>();
    let (pending, retry) = macos_frame_needs_resource_refresh(session, &visible)?;
    let state = session
        .macos_render_host
        .as_mut()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    state.resource_refresh_pending = pending;
    state.resource_retry_pending = retry;
    macos_render_host_snapshot(state, session.session.composition_generation())
}

/// 这一帧的可见资源是否还有未落定的，需要再提交一次去收割 worker 结果。
///
/// 判断此前在平台侧：Swift 每提交一帧就要再查三次——可见 block 列表、全部图片
/// 状态、全部内嵌资源状态——然后自己做集合运算。三次查询的答案全在 Rust 手里，
/// 而且平台还得复制一份状态码的语义表（不变量 I3）。
///
/// 只看可见范围（J3：高成本资源只对当前 viewport 调度）。内嵌资源的
/// `status_for` 同时会推进渲染流水线，因此这里也是 Math/Mermaid 真正被驱动的
/// 地方——此前靠平台的轮询查询顺带驱动，那是个不该由平台承担的职责。
#[cfg(target_os = "macos")]
fn embedded_style_for_block(
    base: yu_assets::EmbeddedStyle,
    theme: yu_core::ThemeId,
    decorations: &yu_editor::BlockDecorations,
) -> Option<yu_assets::EmbeddedStyle> {
    let heading = decorations
        .line_styles()
        .iter()
        .find_map(|ornament| match ornament {
            yu_markdown::BlockOrnament::Heading { level } => Some(*level),
            _ => None,
        });
    let quoted = decorations
        .line_styles()
        .iter()
        .any(|ornament| matches!(ornament, yu_markdown::BlockOrnament::QuoteBar { .. }));
    let (scale, foreground) = if let Some(level) = heading {
        (
            theme.spec().heading_sizes[usize::from(level - 1)],
            theme.heading_color(level),
        )
    } else if quoted {
        (1.0, theme.quote_text_color())
    } else {
        (1.0, base.foreground())
    };
    yu_assets::EmbeddedStyle::new(
        base.font_milli() as f32 / 1000.0 * scale,
        foreground,
        base.dark(),
    )
    .map(|style| style.with_reference_day(base.reference_day()))
}

#[cfg(target_os = "macos")]
fn macos_frame_needs_resource_refresh(
    session: &mut YuStorageSession,
    visible_blocks: &[usize],
) -> Result<(bool, bool), i32> {
    let source = session.session.snapshot();
    let revision = source.revision();
    let definitions = session
        .session
        .document()
        .editor()
        .markdown()
        .reference_definitions()
        .clone();
    let mut pending = false;
    let mut retry = false;
    for &block_index in visible_blocks {
        let decorations = session
            .session
            .block_decorations(block_index)
            .map_err(storage_status)?;
        for image in block_images(&decorations) {
            let key = image_resource_key(&source, image, &definitions);
            let fingerprint = key.as_ref().map_or(0, ImageKey::fingerprint);
            let status = macos_image_resource_status(
                session.macos_render_host.as_ref(),
                key.as_ref(),
                revision.get(),
            );
            // Exhausted failures are settled until an explicit retry or a
            // source revision change. Do not keep waking the renderer for a
            // request that the cache will permanently refuse at this revision.
            if status == YU_STORAGE_IMAGE_RESOURCE_FAILED
                && session.macos_render_host.as_ref().is_some_and(|host| {
                    key.as_ref()
                        .and_then(|key| host.image_resources.cache.failure(key))
                        .is_some_and(|failure| {
                            failure.is_exhausted(host.image_resources.cache.retry_policy())
                        })
                })
            {
                continue;
            }
            if macos_resource_status_needs_refresh(status, fingerprint) {
                pending = true;
                retry |= macos_resource_status_needs_retry(status, fingerprint);
            }
        }
        for resource in decorations
            .widgets()
            .iter()
            .filter_map(|widget| match widget {
                yu_markdown::BlockWidget::Embedded(resource) => Some(*resource),
                _ => None,
            })
        {
            let embedded_kind = match resource.kind {
                yu_markdown::EmbeddedKind::Math => EmbeddedResourceKind::Math,
                yu_markdown::EmbeddedKind::Mermaid => EmbeddedResourceKind::Mermaid,
            };
            let markdown = session.session.document().editor().markdown();
            let content = if resource.kind == yu_markdown::EmbeddedKind::Math {
                match markdown.equation_source(resource) {
                    Ok(content) => content,
                    Err(message) => {
                        let state = &mut session.macos_embedded_resources;
                        if !state.diagnostics.iter().any(|(version, range, old)| {
                            *version == revision && *range == resource.source && *old == message
                        }) {
                            state
                                .diagnostics
                                .retain(|(_, range, _)| *range != resource.source);
                            state.diagnostics.push((revision, resource.source, message));
                            state
                                .publications
                                .retain(|old| old.source_range() != resource.source);
                            yu_render_macos::notify_resource_completion();
                        }
                        pending = true;
                        continue;
                    }
                }
            } else {
                markdown
                    .embedded_source(resource)
                    .ok_or(YU_STORAGE_EDITOR_ERROR)?
            };
            let content = if content.is_empty() {
                "\n".to_owned()
            } else {
                content
            };
            let request =
                EmbeddedRenderRequest::new(revision, resource.source, embedded_kind, content)
                    .map_err(|_| YU_STORAGE_EDITOR_ERROR)?
                    .with_style(
                        embedded_style_for_block(
                            session.macos_embedded_resources.style,
                            session.macos_embedded_resources.theme,
                            &decorations,
                        )
                        .ok_or(YU_STORAGE_EDITOR_ERROR)?
                        .with_display(resource.display),
                    );
            let status = session
                .macos_embedded_resources
                .status_for(request.clone(), revision)?;
            if session
                .macos_embedded_resources
                .cache
                .failure(request.key())
                .is_some_and(|failure| {
                    failure.is_exhausted(session.macos_embedded_resources.cache.retry_policy())
                })
            {
                continue;
            }
            if macos_resource_status_needs_refresh(status, request.key().fingerprint()) {
                pending = true;
                retry |= macos_resource_status_needs_retry(status, request.key().fingerprint());
            }
        }
    }
    Ok((pending, retry))
}

/// A pending resource that is still waiting for a worker completion does not
/// invalidate a retained presentation.  A retryable failure does: the next
/// build must advance the retry clock and re-submit the request.  Completion
/// generations are checked by the caller, so a worker result that arrived
/// since the last build already makes the retained frame ineligible.
const fn macos_pending_resource_allows_retained_reuse(resource_retry_pending: bool) -> bool {
    !resource_retry_pending
}

/// 一个资源的状态是否意味着「还要再取一次」。
///
/// 图片与内嵌资源的状态码在 READY / PENDING / FAILED / UNKNOWN 上取值相同，
/// UNSUPPORTED 只出现在内嵌资源上，因此可以共用这一张表。
///
/// 已就绪与明确不支持都是终态，不再重试。未知状态只在有稳定身份（指纹非零）
/// 时才重试：指纹为零表示这个资源根本没有可调度的目标，重试只会空转。
/// 无法识别的状态码按需要重试处理——宁可多画一帧，也不要停在一个谁也不认识
/// 的状态上。
#[cfg(target_os = "macos")]
const fn macos_resource_status_needs_refresh(status: u8, fingerprint: u64) -> bool {
    match status {
        YU_STORAGE_IMAGE_RESOURCE_READY | YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED => false,
        YU_STORAGE_IMAGE_RESOURCE_UNKNOWN => fingerprint != 0,
        _ => true,
    }
}

#[cfg(target_os = "macos")]
const fn macos_resource_status_needs_retry(status: u8, fingerprint: u64) -> bool {
    status != YU_STORAGE_IMAGE_RESOURCE_PENDING
        && macos_resource_status_needs_refresh(status, fingerprint)
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn pending_resources_wait_for_notifications_not_retry_timers() {
    assert!(macos_resource_status_needs_refresh(
        YU_STORAGE_IMAGE_RESOURCE_PENDING,
        1
    ));
    assert!(!macos_resource_status_needs_retry(
        YU_STORAGE_IMAGE_RESOURCE_PENDING,
        1
    ));
    assert!(macos_resource_status_needs_retry(
        YU_STORAGE_IMAGE_RESOURCE_FAILED,
        1
    ));
    for status in [
        YU_STORAGE_IMAGE_RESOURCE_READY,
        YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED,
    ] {
        assert!(!macos_resource_status_needs_retry(status, 1));
    }
    assert!(!macos_resource_status_needs_retry(
        YU_STORAGE_IMAGE_RESOURCE_UNKNOWN,
        0
    ));
}

#[test]
fn pending_resources_allow_retained_reuse_until_completion_arrives() {
    assert!(macos_pending_resource_allows_retained_reuse(false));
    assert!(!macos_pending_resource_allows_retained_reuse(true));
}

#[cfg(target_os = "macos")]
fn macos_can_reuse_prepared_frame(
    state: &MacosRenderHostState,
    requested: &FrameKey,
    viewport: Rect,
    surface_generation: u64,
    resource_generation: u64,
) -> bool {
    state.resource_completion_generation == resource_generation
        && state.host.surface_generation() == surface_generation
        && macos_pending_resource_allows_retained_reuse(state.resource_retry_pending)
        && (!state.layout_pending
            || state
                .last_frame_key
                .as_ref()
                .is_some_and(|previous| previous != requested))
        && state.prepared_build.as_ref() == Some(requested.build())
        && state.host.frame_handle().is_some_and(|frame| {
            frame.plan().viewport() == viewport
                || macos_frame_covers_viewport(frame.as_ref(), viewport.y(), viewport.height())
        })
}

#[cfg(target_os = "macos")]
fn macos_render_host_surface_prepare(
    session: &mut YuStorageSession,
    view: std::ptr::NonNull<c_void>,
    surface_width: f64,
    surface_height: f64,
    scale: f64,
) -> Result<u64, i32> {
    let config = SurfaceConfig::new(surface_width, surface_height, scale)
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let state = session
        .macos_render_host
        .as_mut()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    if let Some(surface) = state.surface.as_mut() {
        if surface.view != view {
            return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE);
        }
        if surface.surface.config() != config {
            surface
                .surface
                .resize(config)
                .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        }
        return Ok(surface.surface.generation());
    }

    let device = MetalDevice::system_default().map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let surface = MetalSurface::new(device.clone(), config)
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    // SAFETY: the caller has validated that this is a live AppKit view on the
    // main thread, and the adapter explicitly drops the attachment first.
    let attachment = unsafe { surface.attach_to_view_owned(view) }
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let renderer_result = match session.shader_library.as_deref() {
        Some(library) => MetalFrameRenderer::with_library(device.clone(), library),
        None => MetalFrameRenderer::new(device.clone()),
    };
    let renderer = match renderer_result {
        Ok(renderer) => renderer,
        Err(_) => {
            drop(attachment);
            return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE);
        }
    };
    state.surface = Some(MacosPersistentSurfaceState {
        surface,
        attachment: Some(attachment),
        renderer,
        uploader: MetalUploader::new(device.clone()),
        atlas: MetalAtlas::new(),
        image_atlas: MetalImageAtlas::new(),
        view,
    });
    Ok(0)
}

/// Advances the persistent Rust-owned macOS render host through one viewport
/// event. The host retains CoreText shaping, CPU atlas, render-plan
/// fingerprints and revision/surface-generation state across calls. Native
/// code receives only scalar publication metadata; it does not own a second
/// document, atlas or frame cache.
///
/// # Safety
/// `session` must be a live handle and `snapshot` must point to writable
/// storage for one value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_render_host_frame(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    surface_generation: u64,
    appearance: u8,
    snapshot: *mut YuStorageMacosRenderHostSnapshot,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if snapshot.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `snapshot` is a caller-owned output pointer checked above.
    unsafe { *snapshot = YuStorageMacosRenderHostSnapshot::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_generation,
            appearance,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        // 这条诊断查询没有绑定 Metal surface，因此没有 backing scale 可用；
        // 按逻辑尺寸取样即可，它不负责上屏。
        let value = match macos_render_host_frame(
            session,
            MacosFrameRequest {
                expected_revision,
                size,
                max_width,
                scroll_y,
                viewport_height,
                surface_width: 1.0,
                surface_height: 1.0,
                surface_generation,
                raster_scale: 1.0,
                appearance: appearance_from_raw(appearance),
            },
            false,
        ) {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: `snapshot` is a caller-owned output pointer checked above.
        unsafe { *snapshot = value };
        YU_STORAGE_OK
    }
}

/// Submits the persistent host publication to a real AppKit-backed
/// `CAMetalLayer`. The native shell supplies an existing `NSView` pointer and
/// must invoke the synchronous call on the AppKit main thread. Rust lazily
/// creates and then retains the backend-owned surface/renderer/atlas for the
/// same view, while the CoreText publication and host Revision remain
/// persistent on the storage session. The product currently uses the surface
/// as a transparent visual overlay; TextKit remains the input/IME/AX fallback
/// and is not replaced by this bridge.
///
/// The first surface starts at generation zero. A changed surface config
/// resizes that same layer, advances its generation, and lets the host session
/// force the next frame through a full clear. Call
/// `yu_storage_session_macos_render_host_surface_detach` on the AppKit main
/// thread when the view is closing. This bridge remains opt-in and does not
/// remove or replace the production TextKit mirror.
///
/// # Safety
/// `session` must be a live handle, `view` must be a valid main-thread-owned
/// `NSView` for the duration of this synchronous call, and `snapshot` must be
/// writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_render_host_surface_submit(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    surface_width: f64,
    surface_height: f64,
    scale: f64,
    appearance: u8,
    view: *mut c_void,
    snapshot: *mut YuStorageMacosRenderHostSurfaceSnapshot,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if view.is_null() || snapshot.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `snapshot` is a caller-owned output pointer checked above.
    unsafe { *snapshot = YuStorageMacosRenderHostSurfaceSnapshot::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_width,
            surface_height,
            scale,
            appearance,
            view,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        use std::ptr::NonNull;
        let timing_enabled = std::env::var_os("YU_RENDER_TIMING").is_some();
        let timing_start = std::time::Instant::now();

        let Some(view) = NonNull::new(view) else {
            return YU_STORAGE_NULL_POINTER;
        };
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        let has_surface = session
            .macos_render_host
            .as_ref()
            .is_some_and(|state| state.surface.is_some());
        let surface_generation = if has_surface {
            match macos_render_host_surface_prepare(
                session,
                view,
                surface_width,
                surface_height,
                scale,
            ) {
                Ok(generation) => generation,
                Err(status) => return status,
            }
        } else {
            0
        };
        let requested_geometry = FrameGeometry::new(
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_width,
            surface_height,
            scale,
        );
        let requested_key = requested_geometry
            .map(|geometry| frame_key(session, appearance_from_raw(appearance), geometry));
        let presentation_viewport = Rect::new(0.0, scroll_y, max_width, viewport_height).ok();
        let can_reuse = requested_key.as_ref().is_some_and(|requested| {
            session.macos_render_host.as_ref().is_some_and(|state| {
                presentation_viewport.is_some_and(|viewport| {
                    macos_can_reuse_prepared_frame(
                        state,
                        requested,
                        viewport,
                        surface_generation,
                        yu_render_macos::resource_completion_generation(),
                    )
                })
            })
        });
        let (host_snapshot, retained_presentation_viewport) = if can_reuse {
            let Some(state) = session.macos_render_host.as_mut() else {
                return YU_STORAGE_RENDER_HOST_UNAVAILABLE;
            };
            if state
                .host
                .sync_surface_generation(surface_generation)
                .is_err()
            {
                return YU_STORAGE_RENDER_HOST_UNAVAILABLE;
            }
            let snapshot =
                match macos_render_host_snapshot(state, session.session.composition_generation()) {
                    Ok(snapshot) => snapshot,
                    Err(status) => return status,
                };
            (snapshot, presentation_viewport)
        } else {
            let snapshot = match macos_render_host_frame(
                session,
                MacosFrameRequest {
                    expected_revision,
                    size,
                    max_width,
                    scroll_y,
                    viewport_height,
                    surface_width,
                    surface_height,
                    surface_generation,
                    raster_scale: scale as f32,
                    appearance: appearance_from_raw(appearance),
                },
                true,
            ) {
                Ok(snapshot) => snapshot,
                Err(YU_STORAGE_RENDER_BUSY) => {
                    // The worker owns the CPU preparation, so the first call
                    // can return before a surface exists.  Attach the current
                    // surface before returning busy; otherwise every retry
                    // would keep seeing an unbound host and could never reach
                    // Metal submission.
                    let surface_missing = session
                        .macos_render_host
                        .as_ref()
                        .is_none_or(|state| state.surface.is_none());
                    if surface_missing
                        && let Err(status) = macos_render_host_surface_prepare(
                            session,
                            view,
                            surface_width,
                            surface_height,
                            scale,
                        )
                    {
                        return status;
                    }
                    return YU_STORAGE_RENDER_BUSY;
                }
                Err(status) => return status,
            };
            (snapshot, None)
        };
        let frame_build_elapsed = timing_start.elapsed();
        let surface_missing_after_frame = session
            .macos_render_host
            .as_ref()
            .is_none_or(|state| state.surface.is_none());
        if surface_missing_after_frame
            && let Err(status) = macos_render_host_surface_prepare(
                session,
                view,
                surface_width,
                surface_height,
                scale,
            )
        {
            return status;
        }
        let state = match session.macos_render_host.as_mut() {
            Some(state) => state,
            None => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
        };
        let publications = state
            .image_resources
            .publications
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let image_failure_count = state.image_resources.cache.failure_count();
        let image_eviction_count = state.image_resources.cache.eviction_count();
        let image_request_count = state.image_resources.visible_request_count;
        let image_candidate_count = state.image_resources.candidate_count;
        let image_duplicate_count = state.image_resources.duplicate_count;
        let image_visible_candidate_count = state.image_resources.visible_candidate_count;
        let image_overscan_candidate_count = state.image_resources.overscan_candidate_count;
        let image_retry_count = state.image_resources.retry_count;
        let surface_state = match state.surface.as_mut() {
            Some(surface) => surface,
            None => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
        };
        surface_state.image_atlas.retain_publications(&publications);
        let mut uploaded_images = 0_usize;
        for publication in &publications {
            match surface_state
                .image_atlas
                .sync_publication(&mut surface_state.uploader, publication)
            {
                Ok(true) => uploaded_images += 1,
                Ok(false) => {}
                Err(_) => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
            }
        }
        let image_sync_elapsed = timing_start.elapsed();
        if timing_enabled {
            println!(
                "yu-render-metric event=image_upload duration_ms={:.6}",
                image_sync_elapsed
                    .saturating_sub(frame_build_elapsed)
                    .as_secs_f64()
                    * 1000.0
            );
        }
        let submission = match state.host.submit_with_images_at(
            &mut surface_state.renderer,
            &surface_state.surface,
            &mut surface_state.uploader,
            &mut surface_state.atlas,
            &mut surface_state.image_atlas,
            // A worker may have prepared the frame at an earlier scroll origin.
            // Coverage was validated above; encode using the current camera.
            // Geometry is shared with AppKit's input view. The paragraph's x=0
            // is the reading column, not the surface's left edge.
            Rect::new(
                -((surface_width as f32 - max_width) * 0.5),
                scroll_y - resolved_theme_spec(appearance).top,
                surface_width as f32,
                surface_height as f32,
            )
            .ok(),
        ) {
            Ok(submission) => submission,
            Err(error) => return macos_surface_submit_error_status(error),
        };
        if timing_enabled {
            let total = timing_start.elapsed();
            eprintln!(
                "yu-render timing build={:?} image-sync={:?} total={:?} reused={}",
                frame_build_elapsed,
                image_sync_elapsed.saturating_sub(frame_build_elapsed),
                total,
                retained_presentation_viewport.is_some()
            );
        }
        // SAFETY: `snapshot` is a caller-owned output pointer checked above.
        unsafe {
            *snapshot = YuStorageMacosRenderHostSurfaceSnapshot {
                revision: submission.revision().get(),
                composition_generation: host_snapshot.composition_generation,
                surface_generation: submission.surface_generation(),
                frame_serial: submission.frame_serial(),
                uploaded_pages: u64::try_from(submission.uploaded_pages()).unwrap_or(u64::MAX),
                uploaded_images: u64::try_from(uploaded_images).unwrap_or(u64::MAX),
                command_count: host_snapshot.command_count,
                damage_count: host_snapshot.damage_count,
                atlas_page_count: host_snapshot.atlas_page_count,
                image_resource_count: u64::try_from(surface_state.image_atlas.resource_count())
                    .unwrap_or(u64::MAX),
                image_request_count: u64::try_from(image_request_count).unwrap_or(u64::MAX),
                image_failure_count: u64::try_from(image_failure_count).unwrap_or(u64::MAX),
                image_eviction_count,
                image_atlas_eviction_count: surface_state.image_atlas.eviction_count(),
                image_candidate_count: u64::try_from(image_candidate_count).unwrap_or(u64::MAX),
                image_duplicate_count: u64::try_from(image_duplicate_count).unwrap_or(u64::MAX),
                image_visible_candidate_count: u64::try_from(image_visible_candidate_count)
                    .unwrap_or(u64::MAX),
                image_overscan_candidate_count: u64::try_from(image_overscan_candidate_count)
                    .unwrap_or(u64::MAX),
                image_retry_count,
                submitted: 1,
                presentation_reused: u8::from(retained_presentation_viewport.is_some()),
                selection_decoration_count: host_snapshot.selection_decoration_count,
                caret_decoration_count: host_snapshot.caret_decoration_count,
                search_decoration_count: host_snapshot.search_decoration_count,
                highlighted_glyph_count: host_snapshot.highlighted_glyph_count,
                resource_refresh_pending: host_snapshot.resource_refresh_pending,
                resource_retry_pending: host_snapshot.resource_retry_pending,
                layout_pending: host_snapshot.layout_pending,
                content_height: host_snapshot.content_height,
            };
        }
        // 记录这一帧的身份，供 `frame_is_current` 判断后续提交是否等价。
        if let Ok(geometry) = frame_geometry(&YuStorageFrameGeometry {
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_width,
            surface_height,
            scale,
        }) {
            let key = frame_key(session, appearance_from_raw(appearance), geometry);
            if let Some(state) = session.macos_render_host.as_mut() {
                state.last_frame_key = Some(key);
            }
        }
        YU_STORAGE_OK
    }
}

/// Returns the latest drawable's CoreAnimation presentation time, or zero
/// while the current source/geometry has no matching presented submission.
/// # Safety
/// Same live-session and readable/writable pointer contract as frame_is_current.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_frame_presentation_time(
    session: *mut YuStorageSession,
    geometry: *const YuStorageFrameGeometry,
    appearance: u8,
    out_time: *mut f64,
) -> i32 {
    if out_time.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    unsafe { *out_time = 0.0 };
    let mut current = 0_u8;
    let status =
        unsafe { yu_storage_session_frame_is_current(session, geometry, appearance, &mut current) };
    if status != YU_STORAGE_OK {
        return status;
    }
    #[cfg(target_os = "macos")]
    {
        let time = unsafe { &*session }
            .macos_render_host
            .as_ref()
            .and_then(|host| host.surface.as_ref())
            .map_or(0.0, |surface| surface.surface.latest_presentation_time());
        unsafe {
            *out_time = if current != 0 { time } else { 0.0 };
        }
    }
    status
}

/// 判断按给定几何提交的下一帧是否与最近成功提交的帧完全等价。
///
/// 等价的完整定义见 [`FrameKey`]：Revision、composition generation、
/// 全部选区、查询代数、表格 resize 覆盖与几何全部不变才算等价。其中有四项
/// 不推进 Revision，只比 Revision 会把光标移动、加一根光标、preedit 更新、
/// 换查询与列宽拖动全部静默跳过。
///
/// 这个判断此前在平台侧：Swift 每帧先查 Revision、再查 composition
/// generation，才能组装出比较用的键，一次提交因此产生多次纯查询往返。状态在
/// Rust，决策也该在 Rust（不变量 I3）。
///
/// # Safety
/// `session`、`geometry` 与 `out_current` 必须是调用方拥有的有效指针。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_frame_is_current(
    session: *mut YuStorageSession,
    geometry: *const YuStorageFrameGeometry,
    appearance: u8,
    out_current: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if out_current.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // 先把输出置为「不是当前帧」：任何后续失败路径都不能留下陈旧的 1，
    // 那会让平台误以为可以跳过提交（不变量 I4）。
    // SAFETY: `out_current` was checked above and belongs to the caller.
    unsafe { *out_current = 0 };
    let Some(geometry) = (unsafe { geometry.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session, geometry, appearance);
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        let geometry = match frame_geometry(geometry) {
            Ok(geometry) => geometry,
            Err(status) => return status,
        };
        let key = frame_key(session, appearance_from_raw(appearance), geometry);
        let current = session.macos_render_host.as_ref().is_some_and(|state| {
            // 没有 surface 时不能声称当前帧有效：内容还没有真正上屏。
            state.resource_completion_generation
                == yu_render_macos::resource_completion_generation()
                && macos_pending_resource_allows_retained_reuse(state.resource_retry_pending)
                && state.surface.is_some()
                // Resource completion may publish a new frame without changing
                // FrameKey. If Metal is busy, the old on-screen key must not
                // cause Swift to skip the retry for this newer publication.
                && state.host.last_submission().is_some_and(|submission| {
                    Some(submission.frame_serial()) == state.host.frame_serial()
                })
                && state.last_frame_key.as_ref() == Some(&key)
        });
        if !current
            && std::env::var_os("YU_RENDER_TIMING").is_some()
            && let Some(state) = session.macos_render_host.as_ref()
        {
            println!(
                "yu-render-metric event=frame_not_current key={} serial={} retry={} resource={}",
                state.last_frame_key.as_ref() == Some(&key),
                state
                    .host
                    .last_submission()
                    .is_some_and(|s| Some(s.frame_serial()) == state.host.frame_serial()),
                state.resource_retry_pending,
                state.resource_completion_generation
                    == yu_render_macos::resource_completion_generation()
            );
        }
        // SAFETY: `out_current` was checked above and belongs to the caller.
        unsafe { *out_current = u8::from(current) };
        YU_STORAGE_OK
    }
}

/// Detaches and releases the persistent native surface adapter, if one is
/// attached. The call must run on the AppKit main thread so the owned view
/// attachment can restore the previous backing layer safely. It is idempotent.
///
/// # Safety
/// `session` must be null or a live storage handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_render_host_surface_detach(
    session: *mut YuStorageSession,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    // 句柄仍然要校验（null 是错误），但非 macOS 上没有 render host 可解绑。
    #[cfg(not(target_os = "macos"))]
    let _ = session;
    #[cfg(target_os = "macos")]
    if let Some(state) = session.macos_render_host.as_mut() {
        // Invalidate this binding without spawning another preparation thread.
        // A running CoreText call may finish; its result cannot be published.
        if let Some(worker) = state.frame_worker.as_ref() {
            worker.invalidate();
        }
        state.frame_worker_request = None;
        state.binding_generation = match state.binding_generation.checked_add(1) {
            Some(generation) => generation,
            None => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
        };
        state.visible_blocks.clear();
        state.visibility_revision = None;
        state.surface.take();
        state.host = MetalViewportHostSession::new(session.session.revision(), 0);
        // 记录的帧已经随 surface 一起消失，不能留下来让下一次绑定误判等价。
        state.last_frame_key = None;
        state.prepared_build = None;
    }
    YU_STORAGE_OK
}

/// Resolves the current focus caret through the shaped
/// viewport policy. `caret_y` is document-space, while `current_scroll_y` and
/// `target_scroll_y` are absolute document scroll offsets. The native host
/// must apply the target only when the returned Revision still matches.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_shaped_caret_scroll_request(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    output: *mut YuStorageCaretScrollRequest,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageCaretScrollRequest::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
        );
        YU_STORAGE_SHAPER_UNAVAILABLE
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        if !size.is_finite() || size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
            return YU_STORAGE_EDITOR_ERROR;
        }
        let (shaper, metrics, _layout_config) = match macos_query_text_layout(
            session,
            size,
            max_width,
            session.session.viewport_config().layout().theme(),
        ) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(
            session,
            max_width,
            metrics,
            session.session.viewport_config().layout().theme(),
        ) {
            return status;
        }
        // 露出光标时在视口边缘留出的余量。此前由平台传入，而平台算的正是
        // `max(line_height, 4.0)`——一个 Rust 自己就知道的值。为了拿到它，
        // 平台必须先查一次字体度量，于是每次光标移动多一次纯往返。
        let margin = metrics.line_height().max(4.0);
        let focus = session
            .session
            .composition()
            .map_or(session.session.selection().focus(), |overlay| {
                overlay.replacement_range().start()
            });
        if session.session.block_count() > 0
            && let Err(status) = macos_query_layout_snapshot(session, focus, &shaper)
        {
            return status;
        }
        let request = match session.session.caret_scroll_request_with_shaper(
            ViewportSpan::new(scroll_y, viewport_height),
            margin,
            &shaper,
        ) {
            Ok(request) => request,
            Err(error) => return storage_status(error),
        };
        let metadata = match caret_scroll_request_metadata(&session.session, request) {
            Ok(metadata) => metadata,
            Err(status) => return status,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe { *output = metadata };
        YU_STORAGE_OK
    }
}

/// Copies a UTF-16-addressed source range without exposing Rust storage to the
/// native host. The range belongs to `expected_revision` and is suitable for a
/// local native mirror update after a command result reports `SOURCE_SYNC_RANGE`.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_source_range(
    session: *const YuStorageSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let range = match source_range_from_ffi(&session.session, start_utf16, end_utf16) {
        Ok(range) => range,
        Err(status) => return status,
    };
    let snapshot = session.session.snapshot();
    write_snapshot_range(&snapshot, range, output, capacity, written)
}

/// Copies a revision-bound semantic label, interpreting only already-resolved
/// finite HTML. Unsupported markup remains source text.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_accessibility_label(
    session: *const YuStorageSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let range = match source_range_from_ffi(&session.session, start_utf16, end_utf16) {
        Ok(range) => range,
        Err(status) => return status,
    };
    let snapshot = session.session.snapshot();
    let label = session
        .session
        .document()
        .editor()
        .markdown()
        .html_regions()
        .region_for(range)
        .and_then(|region| region.model.as_ref().ok())
        .and_then(|model| model.accessible_text(snapshot.as_str(), range));
    if let Some(label) = label {
        write_bytes(label.as_bytes(), output, capacity, written)
    } else {
        write_snapshot_range(&snapshot, range, output, capacity, written)
    }
}

/// 按指定格式取出当前选区。
///
/// 纯文本与 HTML 片段此前是两个 FFI，参数完全相同，只在输出格式上不同——
/// 剪贴板本来就是「同一段选区的多种表示」。expected revision 让排队中的原生
/// 剪贴板回调不会读到更新的 Revision。
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_selection(
    session: *const YuStorageSession,
    expected_revision: u64,
    format: u8,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if !matches!(
        format,
        YU_STORAGE_CLIPBOARD_TEXT
            | YU_STORAGE_CLIPBOARD_HTML
            | YU_STORAGE_CLIPBOARD_MARKDOWN
            | YU_STORAGE_CLIPBOARD_FRAGMENTS
    ) {
        if !written.is_null() {
            // SAFETY: `written` was checked above and belongs to the caller.
            unsafe { *written = 0 };
        }
        return YU_STORAGE_INVALID_COMMAND;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let range = session.session.selection().ordered_range();
    let snapshot = session.session.snapshot();
    let columns = session.session.selections().table_columns();
    if columns.is_some() && format == YU_STORAGE_CLIPBOARD_TEXT {
        return match session.session.document().editor().copy_table_tsv() {
            Ok(Some(text)) => write_bytes(text.as_bytes(), output, capacity, written),
            Ok(None) => YU_STORAGE_INVALID_SELECTION,
            Err(_) => YU_STORAGE_INVALID_SELECTION,
        };
    }
    if columns.is_some() && format == YU_STORAGE_CLIPBOARD_HTML {
        match session.session.document().editor().copy_html_table_source() {
            Ok(Some(html)) => return write_bytes(html.as_bytes(), output, capacity, written),
            Ok(None) => {}
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        }
    }
    if session.session.selections().is_multiple()
        || columns.is_some()
        || format == YU_STORAGE_CLIPBOARD_FRAGMENTS
    {
        // Copy and cut operate on the same normalized selection set. Empty
        // carets contribute no text; fragments retain source order, regardless
        // of which selection is primary. Newlines separate disjoint fragments.
        let mut fragments = Vec::new();
        let selections = session.session.selections();
        let slots: Vec<_> = selections
            .table_slots()
            .map_or_else(|| (0..selections.len()).map(Some).collect(), <[_]>::to_vec);
        let mut emitted = vec![false; selections.len()];
        for slot in slots.iter().copied() {
            let selection = slot.and_then(|index| {
                let selection = selections.as_slice().get(index)?;
                if std::mem::replace(&mut emitted[index], true) {
                    None
                } else {
                    Some(selection)
                }
            });
            let Some(selection) = selection else {
                fragments.push(String::new());
                continue;
            };
            let range = selection.ordered_range();
            if range.is_empty() && columns.is_none() {
                continue;
            }
            if format != YU_STORAGE_CLIPBOARD_HTML {
                let Some(text) = snapshot
                    .as_str()
                    .get(range.start().get() as usize..range.end().get() as usize)
                else {
                    return YU_STORAGE_INVALID_SELECTION;
                };
                fragments.push(text.to_owned());
            } else if columns.is_some() {
                let Some(text) = snapshot
                    .as_str()
                    .get(range.start().get() as usize..range.end().get() as usize)
                else {
                    return YU_STORAGE_INVALID_SELECTION;
                };
                fragments.push(yu_export::export_html_fragment(text));
            } else {
                match export_clipboard(&snapshot, session.session.revision(), range) {
                    Ok(payload) => fragments.push(payload.html().to_owned()),
                    Err(error) => return status_from_export_error(error),
                }
            }
        }
        if format == YU_STORAGE_CLIPBOARD_FRAGMENTS {
            let source_format = if columns.is_none() {
                YU_STORAGE_FRAGMENT_TEXT
            } else if session.session.document().editor().table_target_is_html() == Some(true) {
                YU_STORAGE_FRAGMENT_HTML
            } else {
                YU_STORAGE_FRAGMENT_MARKDOWN
            };
            let table_source = if columns.is_some()
                && source_format == YU_STORAGE_FRAGMENT_HTML
                && slots.iter().flatten().count() > selections.len()
            {
                match session.session.document().editor().copy_html_table_source() {
                    Ok(source) => source,
                    Err(_) => return YU_STORAGE_INVALID_SELECTION,
                }
            } else {
                None
            };
            let Ok(payload) = serde_json::to_vec(
                &serde_json::json!({"fragments": fragments, "format": source_format, "tableSource": table_source}),
            ) else {
                return YU_STORAGE_INVALID_SELECTION;
            };
            return write_bytes(&payload, output, capacity, written);
        }
        if let Some(columns) = columns {
            let rows: Vec<_> = fragments.chunks(columns).collect();
            let payload = match format {
                YU_STORAGE_CLIPBOARD_HTML => clipboard_table_html(columns, &slots, &fragments),
                YU_STORAGE_CLIPBOARD_MARKDOWN => {
                    let mut lines: Vec<String> = rows
                        .iter()
                        .map(|row| format!("| {} |", row.join(" | ")))
                        .collect();
                    lines.insert(1, format!("| {} |", vec!["---"; columns].join(" | ")));
                    lines.join("\n")
                }
                _ => rows
                    .iter()
                    .map(|row| row.join("\t"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            return write_bytes(payload.as_bytes(), output, capacity, written);
        }
        return write_bytes(fragments.join("\n").as_bytes(), output, capacity, written);
    }
    if format != YU_STORAGE_CLIPBOARD_HTML {
        return write_snapshot_range(&snapshot, range, output, capacity, written);
    }
    let payload = match export_clipboard(&snapshot, session.session.revision(), range) {
        Ok(payload) => payload,
        Err(error) => return status_from_export_error(error),
    };
    write_bytes(payload.html().as_bytes(), output, capacity, written)
}

fn clipboard_table_html(columns: usize, slots: &[Option<usize>], fragments: &[String]) -> String {
    let mut html = String::from("<table>");
    let mut seen = vec![false; slots.len()];
    for (row, cells) in slots.chunks(columns).enumerate() {
        html.push_str("<tr>");
        for (column, owner) in cells.iter().enumerate() {
            let at = row * columns + column;
            let Some(owner) = *owner else {
                html.push_str("<td></td>");
                continue;
            };
            if std::mem::replace(&mut seen[owner], true) {
                continue;
            }
            let width = cells[column..]
                .iter()
                .take_while(|slot| **slot == Some(owner))
                .count();
            let height = slots[at..]
                .iter()
                .step_by(columns)
                .take_while(|slot| **slot == Some(owner))
                .count();
            html.push_str("<td");
            if width > 1 {
                html.push_str(&format!(" colspan=\"{width}\""));
            }
            if height > 1 {
                html.push_str(&format!(" rowspan=\"{height}\""));
            }
            html.push('>');
            html.push_str(&fragments[at]);
            html.push_str("</td>");
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    html
}

/// Converts a trusted-size native `text/html` fragment to Markdown using Yu's
/// strict allowlisted import policy. This function is stateless: it does not
/// read or mutate a document session, and the caller must insert the returned
/// source through the normal revision-bound command API.
///
/// A policy rejection returns `YU_STORAGE_HTML_IMPORT_REJECTED`; native code
/// must then fall back to its `text/plain` payload. The output uses the same
/// two-call owned UTF-8 convention as source queries.
///
/// # Safety
/// `html` must point to a readable UTF-8 buffer of `html_length` bytes (or be
/// null when the length is zero). `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_import_html_fragment(
    html: *const u8,
    html_length: usize,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let html = match read_utf8(html, html_length) {
        Ok(html) => html,
        Err(status) => return status,
    };
    let markdown = match import_html_fragment(html) {
        Ok(markdown) => markdown,
        Err(error) => return status_from_html_import_error(error),
    };
    write_bytes(markdown.as_bytes(), output, capacity, written)
}

/// Returns a compact source-backed Accessibility snapshot. Every coordinate
/// in the result is valid only for `revision`; native queries must use the
/// revision-bound range/copy functions below rather than retaining Rust text.
///
/// # Safety
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_snapshot(
    session: *const YuStorageSession,
    output: *mut YuStorageAccessibilitySnapshot,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    accessibility_snapshot_output(&session.session, output)
}

/// Copies the extended Revision-bound semantic tree. This V2 function keeps
/// the original `yu_storage_session_accessibility_semantic_nodes` struct ABI
/// intact while adding parser-resolved destination and task action metadata.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable V2 nodes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_semantic_nodes_v2(
    session: *const YuStorageSession,
    expected_revision: u64,
    output: *mut YuStorageAccessibilityNodeV2,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let snapshot = match accessibility_semantic_snapshot(&session.session) {
        Ok(snapshot) => snapshot,
        Err(status) => return status,
    };
    write_accessibility_nodes_v2(snapshot.nodes(), output, capacity, written)
}

/// 拷出这一版的大纲：文档里全部标题，按文档顺序（也就是前序），带层级、
/// 面板上那一行文字、跨刷新的身份。
///
/// **两遍协议，两个缓冲区一起**：`items` 与 `text` 都传 null（两个容量都为
/// 0）时只把两个长度写进 `item_count` 与 `text_length`。条目里的
/// `display_utf8_offset` / `identity_utf8_offset` 指进 `text`，所以它们必须
/// 出自同一次调用。
///
/// # 为什么这一版交出的是**文字**，上一版交出的是区间
///
/// 上一版由平台拿自己的 canonical 镜像减掉隐藏区间，理由写在那时的
/// `YuStorageHiddenSpan` 上：「跨边界的是区间，不是文本」。**那条理由针对的
/// 是把整篇文档的视觉字节流搬过去**（那会造出第二份可以与 canonical 漂开的
/// 文档，也正是至今仍然登记着的那条欠账）。一条标题的标签不是那件事：它是
/// 一次派生的、有界的、绑在同一个 Revision 上的显示串，与
/// `yu_storage_session_copy_source_range` 交出选区字节是同一类。
///
/// 换来的是**减法只有一份实现**。上一版那份住在壳里，第二端必须照写第二遍，
/// 而分叉的表现是同一条标题在两端显示得不一样——不报错。
///
/// # Safety
/// `session` must be a live handle. `item_count` and `text_length` must be
/// writable; when non-null, `items` must provide `item_capacity` writable
/// items, `text` must provide `text_capacity` writable bytes, and `styles`
/// must provide `style_capacity` writable runs. All three counts must be writable.
/// Query with all three payload pointers null and capacities zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_outline_items(
    session: *mut YuStorageSession,
    expected_revision: u64,
    items: *mut YuStorageOutlineItem,
    item_capacity: usize,
    item_count: *mut usize,
    text: *mut u8,
    text_capacity: usize,
    text_length: *mut usize,
    styles: *mut YuStorageOutlineStyleRun,
    style_capacity: usize,
    style_count: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let (rows, display, runs) = match outline_rows_output(&mut session.session) {
        Ok(output) => output,
        Err(status) => return status,
    };
    if style_count.is_null() || item_count.is_null() || text_length.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: count pointers are caller-owned writable outputs checked above.
    unsafe {
        *style_count = runs.len();
        *item_count = rows.len();
        *text_length = display.len();
    }
    let measuring = items.is_null()
        && text.is_null()
        && styles.is_null()
        && item_capacity == 0
        && text_capacity == 0
        && style_capacity == 0;
    if !measuring {
        if items.is_null() || text.is_null() {
            return YU_STORAGE_NULL_POINTER;
        }
        if styles.is_null() && (!runs.is_empty() || style_capacity != 0) {
            return YU_STORAGE_NULL_POINTER;
        }
        if style_capacity < runs.len() {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
    }
    let status = write_panel_rows(
        &rows,
        &display,
        &PanelOutput {
            items,
            item_capacity,
            item_count,
            text,
            text_capacity,
            text_length,
        },
    );
    if status == YU_STORAGE_OK && !measuring && !runs.is_empty() {
        // SAFETY: capacity was checked before any payload was copied.
        unsafe {
            ptr::copy_nonoverlapping(runs.as_ptr(), styles, runs.len());
        }
    }
    status
}

/// 换一份搜索查询，立刻在当前这一版源码上扫出全部匹配。
///
/// 空查询留下一份 0 个匹配的状态（面板要靠它显示「没有结果」）；
/// `text` 传 null、`text_length` 传 0 表示**收掉搜索**，高亮一并消失。
///
/// **不校验 Revision。** 查询是与源码正交的一件事：文档在这一刻恰好被外部
/// 改动重载过，正确的行为是在新源码上重扫，而不是把用户刚敲的查询丢掉。
///
/// # Safety
/// `session` must be a live handle. `text` must point at `text_length`
/// readable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_search_query(
    session: *mut YuStorageSession,
    text: *const u8,
    text_length: usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if text.is_null() && text_length == 0 {
        session.session.document_mut().editor_mut().clear_search();
        return YU_STORAGE_OK;
    }
    let query = match read_utf8(text, text_length) {
        Ok(query) => query,
        Err(status) => return status,
    };
    session
        .session
        .document_mut()
        .editor_mut()
        .set_search_query(query);
    YU_STORAGE_OK
}

/// 拷出当前查询在这一版源码上的全部结果行，按文档顺序，互不重叠。
///
/// **两遍协议，两个缓冲区一起**，与 `yu_storage_session_outline_items` 同形。
/// 没有搜索时两个长度都是 0。
///
/// 「命中所在的那一行显示成什么字」由 Rust 算（`yu_editor::SearchResults`）：
/// 取那一行、裁进它所在的块、减掉被藏起来的区间。**裁进块里不是可选的**
/// ——不裁的后果不是画错，是那一行悄悄带回语法标记。
///
/// **「跳到下一个」不在这里。** 它是一次导航，而导航只能有一个实现：平台拿
/// 这个列表挑出光标之后的那一条，再走已有的
/// `yu_storage_session_set_selection_endpoints`。另开一个入口会立刻产生第二个
/// 「怎么跳到一个源码位置」的答案。
///
/// # Safety
/// `session` must be a live handle. `item_count` and `text_length` must be
/// writable; when non-null, `items` must provide `item_capacity` writable
/// items and `text` must provide `text_capacity` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_search_matches(
    session: *mut YuStorageSession,
    expected_revision: u64,
    items: *mut YuStorageSearchMatch,
    item_capacity: usize,
    item_count: *mut usize,
    text: *mut u8,
    text_capacity: usize,
    text_length: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let (rows, display) = match search_rows_output(&mut session.session, expected_revision) {
        Ok(output) => output,
        Err(status) => return status,
    };
    write_panel_rows(
        &rows,
        &display,
        &PanelOutput {
            items,
            item_capacity,
            item_count,
            text,
            text_capacity,
            text_length,
        },
    )
}

/// Returns one logical LF-delimited line range from a source-backed
/// Accessibility snapshot. The line index is zero based and the terminating
/// LF belongs to the preceding line, matching `AccessibilityTextSnapshot`.
///
/// # Safety
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_line_range(
    session: *const YuStorageSession,
    expected_revision: u64,
    line: u64,
    output: *mut YuStorageAccessibilityRange,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    accessibility_line_range_output(&session.session, expected_revision, line, output)
}

/// Resolves a UTF-16 position to its zero-based logical LF-delimited line in
/// the same source-backed Accessibility snapshot.
///
/// # Safety
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_line_for_position(
    session: *const YuStorageSession,
    expected_revision: u64,
    offset_utf16: u64,
    output: *mut u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    accessibility_line_for_position_output(
        &session.session,
        expected_revision,
        offset_utf16,
        output,
    )
}

/// # Safety
///
/// `session` must be null or a live handle and `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_revision(
    session: *const YuStorageSession,
    output: *mut u64,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    unsafe { *output = 0 };
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    // This query reads only canonical in-memory state; it must not inspect disk.
    unsafe { *output = session.session.revision().get() };
    YU_STORAGE_OK
}

/// Reads document state including an explicit disk fingerprint comparison.
/// Use `yu_storage_session_revision` for high-frequency geometry/input queries.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_state(
    session: *const YuStorageSession,
    output: *mut YuStorageState,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let disk_state = match disk_state(&session.session) {
        Ok(value) => value,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe {
        *output = YuStorageState {
            revision: session.session.revision().get(),
            saved_revision: session.session.saved_revision().get(),
            dirty: u8::from(session.session.is_dirty()),
            disk_state,
            bom: match session.session.bom() {
                Utf8Bom::Absent => YU_STORAGE_BOM_ABSENT,
                Utf8Bom::Present => YU_STORAGE_BOM_PRESENT,
            },
            close_state: close_state(session.session.close_state()),
        };
    }
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle; all output pointers must be
/// writable when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_save(
    session: *mut YuStorageSession,
    revision_output: *mut u64,
    bytes_written_output: *mut usize,
    changed_output: *mut u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if revision_output.is_null() || bytes_written_output.is_null() || changed_output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let outcome = match session.session.save() {
        Ok(outcome) => outcome,
        Err(error) => return status_from_error(error),
    };
    let (revision, bytes_written, changed) = match outcome {
        SaveOutcome::Saved {
            revision,
            bytes_written,
        } => (revision.get(), bytes_written, 1),
        SaveOutcome::Unchanged { revision } => (revision.get(), 0, 0),
    };
    // SAFETY: all output pointers were checked above.
    unsafe {
        *revision_output = revision;
        *bytes_written_output = bytes_written;
        *changed_output = changed;
    }
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle and `revision_output` must be
/// writable when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_reload(
    session: *mut YuStorageSession,
    revision_output: *mut u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if revision_output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let outcome = match session.session.reload() {
        Ok(outcome) => outcome,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `revision_output` was checked and belongs to the caller.
    unsafe { *revision_output = outcome.revision.get() };
    YU_STORAGE_OK
}

/// # Safety
///
/// `session` must be null or a live handle and `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_request_close(
    session: *mut YuStorageSession,
    output: *mut YuStorageCloseRequest,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let result = match session.session.close_request() {
        Ok(CloseRequest::CloseNow) => YU_STORAGE_CLOSE_NOW,
        Ok(CloseRequest::Prompt(_)) => YU_STORAGE_CLOSE_PROMPT,
        Ok(CloseRequest::AlreadyClosed) => YU_STORAGE_CLOSE_ALREADY_CLOSED,
        Err(error) => return status_from_error(error),
    };
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe {
        *output = YuStorageCloseRequest {
            result,
            close_state: close_state(session.session.close_state()),
        };
    }
    YU_STORAGE_OK
}

/// 结束一次关闭协商。
///
/// cancel / save / discard 此前是三个独立 FFI：同样的 session 参数、同样的
/// 前置状态、同样的返回码，只在「怎么收场」上不同。它们是同一个关闭协商的
/// 三个出口，属于不变量 I3 的「文件操作」一类，一个带 action 的入口就够了。
///
/// 协商本身仍由 `request_close` 发起——它是查询，返回是否需要向用户提问。
///
/// # Safety
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_close_resolve(
    session: *mut YuStorageSession,
    action: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    match action {
        YU_STORAGE_CLOSE_RESOLVE_ABORT => {
            session.session.abort_close();
            YU_STORAGE_OK
        }
        YU_STORAGE_CLOSE_RESOLVE_CANCEL => session
            .session
            .cancel_close()
            .map_or(YU_STORAGE_INVALID_STATE, |_| YU_STORAGE_OK),
        YU_STORAGE_CLOSE_RESOLVE_SAVE => match session.session.save_close() {
            Ok(_) => YU_STORAGE_OK,
            Err(error) => status_from_error(error),
        },
        YU_STORAGE_CLOSE_RESOLVE_DISCARD => session
            .session
            .discard_close()
            .map_or(YU_STORAGE_INVALID_STATE, |_| YU_STORAGE_OK),
        _ => YU_STORAGE_INVALID_COMMAND,
    }
}

/// Resolved native theme; this value is also consumed by Rust layout/painting.
fn resolved_theme_spec(value: u8) -> yu_core::ThemeSpec {
    match value {
        YU_STORAGE_APPEARANCE_DARK => yu_core::ThemeSpec::NIGHT,
        YU_STORAGE_THEME_YU_LIGHT => yu_core::ThemeSpec::YU_LIGHT,
        YU_STORAGE_THEME_YU_DARK => yu_core::ThemeSpec::YU_DARK,
        _ => yu_core::ThemeSpec::GITHUB,
    }
}

pub type YuStorageThemeSpec = yu_core::ThemeSpec;
pub type YuStorageReadingGeometry = yu_core::ReadingGeometry;

/// # Safety
/// `output` must be writable for one ThemeSpec.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_theme_spec(
    appearance: u8,
    output: *mut YuStorageThemeSpec,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let theme = resolved_theme_spec(appearance);
    unsafe {
        *output = theme;
    }
    YU_STORAGE_OK
}

/// # Safety
/// `output` must be writable for one ReadingGeometry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_reading_geometry(
    appearance: u8,
    width: f32,
    window_width: f32,
    scroll_y: f32,
    column_width: f32,
    output: *mut YuStorageReadingGeometry,
) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if !width.is_finite()
        || width <= 0.0
        || !window_width.is_finite()
        || window_width <= 0.0
        || !scroll_y.is_finite()
        || !column_width.is_finite()
        || column_width < 0.0
    {
        return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
    }
    unsafe {
        let theme = resolved_theme_spec(appearance);
        *output = theme.reading_geometry_with_column(
            width,
            window_width,
            scroll_y,
            (column_width > 0.0).then_some(column_width),
        );
    }
    YU_STORAGE_OK
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use yu_core::ByteOffset;

    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[cfg(target_os = "macos")]
    #[test]
    fn disclosure_image_requests_follow_visible_nested_content_without_losing_inventory() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yu-disclosure-images-{}-{id}.md",
            std::process::id()
        ));
        let source = "<details><summary>Outer<img src='summary.png'></summary><p><img src='outer.png'></p><details><summary>Inner</summary><p><img src='inner.png'></p></details></details>";
        fs::write(&path, source).expect("fixture");
        let mut session = new_storage_session(DocumentEditorSession::open(&path).expect("open"));
        fn requests(session: &mut YuStorageSession) -> Vec<String> {
            let blocks = (0..session
                .session
                .document()
                .editor()
                .markdown()
                .blocks()
                .len())
                .map(|index| (index, ImageRequestPriority::Visible))
                .collect::<Vec<_>>();
            macos_image_requests(session, &blocks)
                .expect("plan")
                .requests()
                .iter()
                .map(|request| request.key().destination().to_owned())
                .collect()
        }
        fn toggle(session: &mut YuStorageSession, label: &str) {
            let at = session
                .session
                .snapshot()
                .as_str()
                .find(label)
                .expect("summary");
            session
                .session
                .execute(EditorCommand::ToggleHtmlDetails {
                    source: ByteOffset::new(at as u64),
                })
                .expect("toggle");
        }
        assert_eq!(
            session
                .session
                .document()
                .editor()
                .image_references()
                .expect("inventory")
                .len(),
            3
        );
        assert_eq!(requests(&mut session), ["summary.png"]);
        toggle(&mut session, "Outer");
        let open = requests(&mut session);
        assert_eq!(open.len(), 2);
        assert!(open.contains(&"summary.png".to_owned()));
        assert!(open.contains(&"outer.png".to_owned()));
        toggle(&mut session, "Inner");
        assert_eq!(requests(&mut session).len(), 3);
        toggle(&mut session, "Outer");
        assert_eq!(requests(&mut session), ["summary.png"]);
        assert_eq!(
            session
                .session
                .document()
                .editor()
                .image_references()
                .expect("inventory")
                .len(),
            3
        );
        session
            .session
            .execute(EditorCommand::Undo)
            .expect("undo close");
        assert_eq!(requests(&mut session).len(), 3);
        session
            .session
            .execute(EditorCommand::Undo)
            .expect("undo inner");
        assert_eq!(requests(&mut session).len(), 2);
        session
            .session
            .execute(EditorCommand::Undo)
            .expect("undo outer");
        assert_eq!(requests(&mut session), ["summary.png"]);
        assert_eq!(session.session.snapshot().as_str(), source);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn navigation_reveal_ffi_preserves_source_revision_and_rejects_invalid_queries() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-reveal-{}-{id}.md", std::process::id()));
        let source = "<details><summary>Summary</summary><p>🙂Target</p></details>";
        fs::write(&path, source).expect("fixture");
        let mut session = new_storage_session(DocumentEditorSession::open(&path).expect("open"));
        let raw = session.as_mut() as *mut YuStorageSession;
        let revision = session.session.revision().get();
        let at = source[..source.find("🙂").expect("emoji")]
            .encode_utf16()
            .count() as u64;
        let mut changed = 0;
        assert_ne!(
            unsafe {
                yu_storage_session_reveal_source_range(raw, revision + 1, at, at, &mut changed)
            },
            YU_STORAGE_OK
        );
        assert_ne!(
            unsafe {
                yu_storage_session_reveal_source_range(raw, revision, at + 1, at + 1, &mut changed)
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_reveal_source_range(raw, revision, at, at, std::ptr::null_mut())
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(
            unsafe { yu_storage_session_reveal_source_range(raw, revision, at, at, &mut changed) },
            YU_STORAGE_OK
        );
        assert_eq!(changed, 1);
        assert_eq!(session.session.revision().get(), revision);
        assert_eq!(session.session.snapshot().as_str(), source);
        assert_eq!(
            session
                .session
                .document()
                .editor()
                .history_stats()
                .undo_entries(),
            0
        );
        assert_eq!(
            unsafe { yu_storage_session_reveal_source_range(raw, revision, at, at, &mut changed) },
            YU_STORAGE_OK
        );
        assert_eq!(changed, 0);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn disclosure_ffi_binds_visible_headers_to_revision_and_preserves_source_on_errors() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("yu-disclosure-{}-{id}.md", std::process::id()));
        let source = "羽🙂\r\n\r\n<details><summary>Title</summary><p>Body</p></details>\r\n";
        fs::write(&path, source).expect("fixture");
        let mut session = new_storage_session(DocumentEditorSession::open(&path).expect("open"));
        let raw = session.as_mut() as *mut YuStorageSession;
        let revision = session.session.revision().get();
        let at = source[..source.find("Title").expect("title")]
            .encode_utf16()
            .count() as u64;
        let mut header = YuStorageAccessibilityRange::default();
        let mut open = 9;
        assert_eq!(
            unsafe {
                yu_storage_session_disclosure_header(raw, revision, at, &mut header, &mut open)
            },
            YU_STORAGE_OK
        );
        assert!(header.start_utf16 < header.end_utf16);
        assert_eq!(open, 0);
        let mut output = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_toggle_disclosure(raw, revision, at, std::ptr::null_mut())
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_ne!(
            unsafe { yu_storage_session_toggle_disclosure(raw, revision, 2, &mut output) },
            YU_STORAGE_OK
        );
        assert_eq!(session.session.snapshot().as_str(), source);
        assert_eq!(
            unsafe { yu_storage_session_toggle_disclosure(raw, revision, at, &mut output) },
            YU_STORAGE_OK
        );
        let expected = source.replace("<details>", "<details open>");
        assert_eq!(session.session.snapshot().as_str(), expected);
        assert_ne!(
            unsafe { yu_storage_session_toggle_disclosure(raw, revision, at, &mut output) },
            YU_STORAGE_OK
        );
        assert_eq!(session.session.snapshot().as_str(), expected);
        let current = session.session.revision().get();
        assert_eq!(
            unsafe {
                yu_storage_session_disclosure_header(
                    raw,
                    current,
                    header.start_utf16,
                    &mut header,
                    &mut open,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(open, 1);
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(raw, YU_STORAGE_COMMAND_UNDO, 0, &mut output)
            },
            YU_STORAGE_OK
        );
        assert_eq!(session.session.snapshot().as_str(), source);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn link_destination_ffi_validates_revision_utf16_and_buffer_contract() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-links-{}-{id}.md", std::process::id()));
        let source = "羽🙂 [文档](https://example.com/?a=1&amp;b=2)\r\n\r\n<div><a href='#part'>章节</a><p id='part'>Target</p></div>\r\n";
        fs::write(&path, source).expect("fixture");
        let mut session = new_storage_session(DocumentEditorSession::open(&path).expect("open"));
        let raw = session.as_mut() as *mut YuStorageSession;
        let revision = session.session.revision().get();
        for (label, expected) in [("文档", "https://example.com/?a=1&b=2"), ("章节", "#part")] {
            let at = source[..source.find(label).expect("label")]
                .encode_utf16()
                .count() as u64;
            let mut written = 0;
            assert_eq!(
                unsafe {
                    yu_storage_session_copy_link_destination(
                        raw,
                        revision,
                        at,
                        std::ptr::null_mut(),
                        0,
                        &mut written,
                    )
                },
                YU_STORAGE_OK
            );
            assert_eq!(written, expected.len());
            let mut output = vec![0; written];
            assert_eq!(
                unsafe {
                    yu_storage_session_copy_link_destination(
                        raw,
                        revision,
                        at,
                        output.as_mut_ptr(),
                        output.len() - 1,
                        &mut written,
                    )
                },
                YU_STORAGE_BUFFER_TOO_SMALL
            );
            assert_eq!(
                unsafe {
                    yu_storage_session_copy_link_destination(
                        raw,
                        revision,
                        at,
                        output.as_mut_ptr(),
                        output.len(),
                        &mut written,
                    )
                },
                YU_STORAGE_OK
            );
            assert_eq!(&output[..written], expected.as_bytes());
            assert_ne!(
                unsafe {
                    yu_storage_session_copy_link_destination(
                        raw,
                        revision + 1,
                        at,
                        output.as_mut_ptr(),
                        output.len(),
                        &mut written,
                    )
                },
                YU_STORAGE_OK
            );
        }
        let mut written = 99;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_link_destination(
                    raw,
                    revision,
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, 0);
        // UTF-16 offset 2 falls between the two surrogate units of the emoji.
        assert_ne!(
            unsafe {
                yu_storage_session_copy_link_destination(
                    raw,
                    revision,
                    2,
                    std::ptr::null_mut(),
                    0,
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        let at = source[..source.find("章节").expect("label")]
            .encode_utf16()
            .count() as u64;
        let expected = source[..source.find("<p id='part'>").expect("id")]
            .encode_utf16()
            .count() as u64;
        let mut target = YuStorageAccessibilityRange::default();
        assert_eq!(
            unsafe { yu_storage_session_document_reference_target(raw, revision, at, &mut target) },
            YU_STORAGE_OK
        );
        assert_eq!(target.start_utf16, expected);
        assert!(target.end_utf16 > target.start_utf16);
        assert_eq!(target.revision, revision);
        drop(session);
        fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn footnote_ffi_navigation_and_diagnostics_use_current_utf16_source() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-footnote-{}-{id}.md", std::process::id()));
        let source = "羽🙂[^注] [^missing]\r\n\r\n[^注]: definition\r\n";
        fs::write(&path, source).expect("fixture");
        let mut session = new_storage_session(DocumentEditorSession::open(&path).expect("open"));
        let raw = session.as_mut() as *mut YuStorageSession;
        let revision = session.session.revision().get();
        let utf16 = |byte| source[..byte].encode_utf16().count() as u64;
        let reference = source.find("[^注]").expect("reference");
        let definition = source.rfind("[^注]").expect("definition");
        let mut target = YuStorageAccessibilityRange::default();
        assert_eq!(
            unsafe {
                yu_storage_session_document_reference_target(
                    raw,
                    revision,
                    utf16(reference),
                    &mut target,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(target.start_utf16, utf16(definition));
        assert_eq!(target.revision, revision);
        assert_eq!(
            unsafe {
                yu_storage_session_document_reference_target(
                    raw,
                    revision,
                    utf16(definition),
                    &mut target,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            (target.start_utf16, target.end_utf16),
            (utf16(reference), utf16(reference + "[^注]".len()))
        );
        assert_ne!(
            unsafe {
                yu_storage_session_document_reference_target(
                    raw,
                    revision + 1,
                    utf16(reference),
                    &mut target,
                )
            },
            YU_STORAGE_OK
        );
        let missing = utf16(source.find("[^missing]").expect("missing"));
        let mut bytes = vec![0; 256];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_document_diagnostic(
                    raw,
                    revision,
                    missing,
                    bytes.as_mut_ptr(),
                    bytes.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert!(
            std::str::from_utf8(&bytes[..written])
                .expect("UTF-8")
                .contains("找不到")
        );
        assert_eq!(session.session.snapshot().as_str(), source);
        drop(session);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn spelling_ffi_binds_ranges_and_replacements_to_revision() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-spelling-{}-{id}.md", std::process::id()));
        fs::write(&path, "中文 wrld `codde`\r\n").expect("fixture");
        let mut session = new_storage_session(DocumentEditorSession::open(&path).expect("open"));
        let raw = session.as_mut() as *mut YuStorageSession;
        let revision = session.session.revision().get();
        let mut count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_spelling_ranges(
                    raw,
                    revision,
                    0,
                    17,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(count, 1);
        let mut records = vec![YuStorageAccessibilityRange::default(); count];
        assert_eq!(
            unsafe {
                yu_storage_session_spelling_ranges(
                    raw,
                    revision,
                    0,
                    17,
                    records.as_mut_ptr(),
                    count,
                    &mut count,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(records[0].start_utf16, 0);
        assert!(records[0].end_utf16 <= 8);
        let info = YuStorageAccessibilityRange {
            revision,
            start_utf16: 3,
            end_utf16: 7,
        };
        assert_eq!(
            unsafe { yu_storage_session_set_spelling_diagnostics(raw, revision, &info, 1) },
            YU_STORAGE_OK
        );
        let before = session.session.document().editor().spelling_generation();
        assert_eq!(
            unsafe { yu_storage_session_set_spelling_diagnostics(raw, revision, ptr::null(), 1) },
            YU_STORAGE_NULL_POINTER
        );
        let invalid = YuStorageAccessibilityRange {
            revision,
            start_utf16: 9,
            end_utf16: 14,
        };
        assert_ne!(
            unsafe { yu_storage_session_set_spelling_diagnostics(raw, revision, &invalid, 1) },
            YU_STORAGE_OK
        );
        assert_eq!(
            session.session.document().editor().spelling_generation(),
            before
        );
        let mut output = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_replace_spelling(
                    raw,
                    &info,
                    b"world".as_ptr(),
                    5,
                    ptr::null_mut(),
                )
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(session.session.revision().get(), revision);
        assert_eq!(
            unsafe {
                yu_storage_session_replace_spelling(raw, &info, b"world".as_ptr(), 5, &mut output)
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            session.session.snapshot().as_str(),
            "中文 world `codde`\r\n"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_replace_spelling(raw, &info, b"wrong".as_ptr(), 5, &mut output)
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_spelling_ranges(
                    raw,
                    revision,
                    0,
                    17,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe { yu_storage_session_set_spelling_diagnostics(raw, revision, &info, 1) },
            YU_STORAGE_STALE_REVISION
        );
        assert!(
            session
                .session
                .document()
                .editor()
                .spelling_diagnostics()
                .is_empty()
        );
        drop(session);
        fs::remove_file(path).expect("cleanup");
    }

    /// 临时文件名里的那一段。
    ///
    /// **进程内的计数器不够**：`$TMPDIR` 是同机共享的，同时跑两个这个测试
    /// 二进制（一轮被杀而 `cargo test` 的子进程没死，就会发生）就必然撞名，
    /// 表现是十几条用例一起炸在 `fs::remove_file` 的 `NotFound` 上——看上去
    /// 完全像一次真的回归。加上 pid 就根治了。
    fn temp_id() -> String {
        format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[test]
    fn file_lifecycle_ffi_preserves_recovery_baseline_and_save_as_history() {
        let directory = std::env::temp_dir().join(format!("yu-file-lifecycle-{}", temp_id()));
        fs::create_dir_all(&directory).expect("directory");
        let draft = directory.join("draft.md");
        let saved = directory.join("saved.md");
        let root = directory.join("recovery");
        let draft_bytes = draft.to_str().expect("path").as_bytes();
        let saved_bytes = saved.to_str().expect("path").as_bytes();
        let root_bytes = root.to_str().expect("path").as_bytes();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_create(draft_bytes.as_ptr(), draft_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert!(!draft.exists());
        let session = unsafe { &mut *raw };
        session
            .session
            .execute(EditorCommand::insert_text("中文🙂\r\n"))
            .expect("edit");
        assert_eq!(
            unsafe { yu_storage_session_recovery(raw, root_bytes.as_ptr(), root_bytes.len(), 0) },
            YU_STORAGE_OK
        );
        let record_path = RecoveryStore::new(&root).path_for(&draft).expect("path");
        let record_bytes = record_path.to_str().expect("path").as_bytes();
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_recovery_copy_target(
                    record_bytes.as_ptr(),
                    record_bytes.len(),
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        let mut target = vec![0; required];
        assert_eq!(
            unsafe {
                yu_storage_recovery_copy_target(
                    record_bytes.as_ptr(),
                    record_bytes.len(),
                    target.as_mut_ptr(),
                    target.len(),
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(target, draft_bytes);
        let mut recovered = ptr::null_mut();
        assert_eq!(
            unsafe {
                yu_storage_session_open_recovery(
                    record_bytes.as_ptr(),
                    record_bytes.len(),
                    &mut recovered,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { &*recovered }.session.snapshot().as_str(),
            "中文🙂\r\n"
        );
        assert!(unsafe { &*recovered }.session.is_dirty());
        assert_eq!(
            unsafe { yu_storage_session_save_as(raw, saved_bytes.as_ptr(), saved_bytes.len(), 0) },
            YU_STORAGE_OK
        );
        assert_eq!(fs::read(&saved).expect("read"), "中文🙂\r\n".as_bytes());
        assert!(!draft.exists());
        let original = unsafe { &mut *raw };
        original
            .session
            .execute(EditorCommand::Undo)
            .expect("history survived");
        assert_eq!(original.session.snapshot().as_str(), "");
        assert_eq!(
            unsafe {
                yu_storage_session_save_as(recovered, saved_bytes.as_ptr(), saved_bytes.len(), 0)
            },
            YU_STORAGE_EXTERNAL_CHANGE
        );
        assert_eq!(unsafe { &*recovered }.session.path(), &draft);
        let mut request = YuStorageCloseRequest::default();
        assert_eq!(
            unsafe { yu_storage_session_request_close(recovered, &mut request) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_close_resolve(recovered, YU_STORAGE_CLOSE_RESOLVE_DISCARD)
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_close_resolve(recovered, YU_STORAGE_CLOSE_RESOLVE_ABORT) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_request_close(recovered, &mut request) },
            YU_STORAGE_OK
        );
        assert_eq!(request.result, YU_STORAGE_CLOSE_PROMPT);
        assert_eq!(
            unsafe {
                yu_storage_session_recovery(recovered, root_bytes.as_ptr(), root_bytes.len(), 1)
            },
            YU_STORAGE_OK
        );
        assert!(!record_path.exists());
        unsafe {
            yu_storage_session_destroy(raw);
            yu_storage_session_destroy(recovered);
        }
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn file_lifecycle_ffi_rejects_null_invalid_actions_and_corrupt_recovery() {
        assert_eq!(
            unsafe { yu_storage_session_create(ptr::null(), 0, ptr::null_mut()) },
            YU_STORAGE_NULL_POINTER
        );
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_create(ptr::null(), 0, &mut raw) },
            YU_STORAGE_INVALID_PATH
        );
        assert!(raw.is_null());
        assert_eq!(
            unsafe { yu_storage_session_save_as(raw, ptr::null(), 0, 0) },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(
            unsafe { yu_storage_session_recovery(raw, ptr::null(), 0, 0) },
            YU_STORAGE_NULL_POINTER
        );
        let path = std::env::temp_dir().join(format!("yu-bad-recovery-{}", temp_id()));
        fs::write(&path, b"not a recovery file").expect("corrupt file");
        let bytes = path.to_str().expect("path").as_bytes();
        assert_eq!(
            unsafe { yu_storage_session_open_recovery(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_INVALID_RECOVERY
        );
        assert!(raw.is_null());
        assert_eq!(fs::read(&path).expect("preserved"), b"not a recovery file");
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_table_width_store_restores_a_new_session() {
        let directory = std::env::temp_dir().join(format!("yu-ffi-widths-{}", temp_id()));
        fs::create_dir_all(&directory).expect("directory");
        let path = directory.join("document.md");
        let source = "| A | B |\n| --- | --- |\n| one | two |\n";
        fs::write(&path, source).expect("source");
        let root = directory.join("state");
        let root_bytes = root.to_string_lossy().as_bytes().to_vec();
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_table_width_store(raw, root_bytes.as_ptr(), root_bytes.len())
            },
            YU_STORAGE_OK
        );
        let editor = unsafe { &mut *raw }.session.document_mut().editor_mut();
        let mut layout = editor
            .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
            .expect("layout");
        layout.apply_table_column_resize(0, -50.0).expect("resize");
        editor
            .confirm_table_column_widths(0, layout.table().expect("table"))
            .expect("confirm");
        let records = editor.table_column_width_records();
        assert_eq!(
            unsafe { yu_storage_session_persist_table_widths(raw) },
            YU_STORAGE_OK
        );
        unsafe { yu_storage_session_destroy(raw) };
        raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_table_width_store(raw, root_bytes.as_ptr(), root_bytes.len())
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { &*raw }
                .session
                .document()
                .editor()
                .table_column_width_records(),
            records
        );
        assert!(!unsafe { &*raw }.session.document().is_dirty());
        assert_eq!(fs::read_to_string(path).expect("source unchanged"), source);
        unsafe { yu_storage_session_destroy(raw) };
        assert_eq!(
            unsafe { yu_storage_session_persist_table_widths(ptr::null_mut()) },
            YU_STORAGE_NULL_POINTER
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reference_day_cancels_old_resources_without_editing_document() {
        let path = std::env::temp_dir().join(format!("yu-calendar-{}.md", temp_id()));
        let source = "# 日期\n\n```mermaid\ngantt\nTask :1d\n```\n";
        fs::write(&path, source).expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let old = unsafe { &*raw }.macos_embedded_resources.control.clone();
        assert!(old.is_current(0));
        assert_eq!(
            unsafe { yu_storage_session_macos_set_reference_day(raw, 20454) },
            YU_STORAGE_OK
        );
        assert!(!old.is_current(0));
        let current = unsafe { &*raw }.macos_embedded_resources.control.clone();
        assert_eq!(
            unsafe { yu_storage_session_macos_set_reference_day(raw, 20454) },
            YU_STORAGE_OK
        );
        assert!(current.is_current(0), "same day must retain resource jobs");
        assert_eq!(
            unsafe { yu_storage_session_macos_set_reference_day(raw, 20455) },
            YU_STORAGE_OK
        );
        assert!(!current.is_current(0));
        assert_eq!(
            unsafe { yu_storage_session_macos_set_reference_day(raw, i32::MAX) },
            YU_STORAGE_EDITOR_ERROR
        );
        let session = unsafe { &*raw };
        assert_eq!(
            session.macos_embedded_resources.style.reference_day(),
            Some(20455)
        );
        assert_eq!(session.session.snapshot().as_str(), source);
        assert_eq!(session.session.snapshot().revision(), Revision::INITIAL);
        assert!(!session.session.document().is_dirty());
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn status_and_state_contracts_are_stable() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-{id}.md"));
        fs::write(&path, "羽 日本語 🙂\n").expect("fixture");
        let editor_session = DocumentEditorSession::open(&path).expect("open");
        let session = YuStorageSession {
            focus_mode: false,
            session: editor_session,
            table_resize_gesture: None,
            table_resize_override: None,
            #[cfg(target_os = "macos")]
            macos_render_host: None,
            #[cfg(target_os = "macos")]
            shader_library: None,
            #[cfg(target_os = "macos")]
            macos_embedded_resources: MacosEmbeddedResourceState::new(),
        };
        assert_eq!(
            close_state(session.session.close_state()),
            YU_STORAGE_CLOSE_OPEN
        );
        assert_eq!(
            disk_state(&session.session).expect("disk state"),
            YU_STORAGE_DISK_UNCHANGED
        );
        assert_eq!(session.session.snapshot().as_str(), "羽 日本語 🙂\n");
        let _ = fs::remove_file(path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn embedded_query_returns_pending_until_worker_completes() {
        let (jobs, queued) = std::sync::mpsc::channel();
        let (completed, results) = std::sync::mpsc::channel();
        let mut state = MacosEmbeddedResourceState {
            cache: EmbeddedResourceCache::new(),
            jobs,
            results,
            control: RenderControl::default(),
            style: yu_assets::EmbeddedStyle::default(),
            theme: yu_core::ThemeId::Github,
            diagnostics: Vec::new(),
            publications: Vec::new(),
        };
        let request = EmbeddedRenderRequest::new(
            Revision::INITIAL,
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(3))
                .expect("valid FFI session fixture"),
            EmbeddedResourceKind::Math,
            "x^2",
        )
        .expect("valid FFI session fixture");
        assert_eq!(
            state
                .status_for(request.clone(), Revision::INITIAL)
                .expect("valid FFI session fixture"),
            YU_STORAGE_EMBEDDED_RESOURCE_PENDING
        );
        let job = queued.try_recv().expect("valid FFI session fixture");
        assert_eq!(
            state
                .status_for(request.clone(), Revision::INITIAL)
                .expect("valid FFI session fixture"),
            YU_STORAGE_EMBEDDED_RESOURCE_PENDING
        );
        assert!(queued.try_recv().is_err(), "duplicate job while in flight");
        let payload = Ok(yu_assets::EmbeddedRenderPayload::svg(
            20,
            20,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"/>",
        )
        .expect("fixture"));
        completed
            .send((job, payload))
            .expect("valid FFI session fixture");
        assert_eq!(
            state
                .status_for(request, Revision::INITIAL)
                .expect("valid FFI session fixture"),
            YU_STORAGE_EMBEDDED_RESOURCE_READY
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn formula_style_inherits_heading_size_and_quote_color() {
        for theme in [
            yu_core::ThemeId::Github,
            yu_core::ThemeId::Night,
            yu_core::ThemeId::YuLight,
            yu_core::ThemeId::YuDark,
        ] {
            let base = yu_assets::EmbeddedStyle::new(16.0, theme.spec().text, false)
                .expect("base")
                .with_reference_day(Some(20454));
            let mut heading = yu_editor::EditorDocument::new("# Title $x$\n");
            let style = embedded_style_for_block(
                base,
                theme,
                heading.block_decorations(0).expect("heading"),
            )
            .expect("style");
            assert_eq!(style.reference_day(), base.reference_day());
            assert_eq!(
                style.font_milli(),
                (16_000.0 * theme.spec().heading_sizes[0]).round() as u32
            );
            assert_eq!(style.foreground(), theme.heading_color(1));
            let mut quote = yu_editor::EditorDocument::new("> text $x$\n");
            let style =
                embedded_style_for_block(base, theme, quote.block_decorations(0).expect("quote"))
                    .expect("style");
            assert_eq!(style.font_milli(), 16_000);
            assert_eq!(style.foreground(), theme.quote_text_color());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn embedded_failures_keep_diagnostics_only_for_current_revision() {
        let (jobs, queued) = std::sync::mpsc::channel();
        let (completed, results) = std::sync::mpsc::channel();
        let mut state = MacosEmbeddedResourceState {
            cache: EmbeddedResourceCache::new(),
            jobs,
            results,
            control: RenderControl::default(),
            style: yu_assets::EmbeddedStyle::default(),
            theme: yu_core::ThemeId::Github,
            diagnostics: Vec::new(),
            publications: Vec::new(),
        };
        let range = TextRange::new(ByteOffset::ZERO, ByteOffset::new(3)).expect("range");
        let request =
            EmbeddedRenderRequest::new(Revision::INITIAL, range, EmbeddedResourceKind::Math, "bad")
                .expect("request");
        state
            .status_for(request, Revision::INITIAL)
            .expect("schedule");
        let job = queued.try_recv().expect("job");
        completed
            .send((
                job.clone(),
                Err(RenderFailure::InvalidSource("unknown command".into())),
            ))
            .expect("failure");
        state.advance(Revision::INITIAL).expect("completion");
        assert_eq!(
            state.diagnostics,
            vec![(Revision::INITIAL, range, "unknown command".into())]
        );
        assert!(state.publications.is_empty());
        let next = Revision::new(Revision::INITIAL.get() + 1);
        state.advance(next).expect("revision advance");
        assert!(state.diagnostics.is_empty());
        completed
            .send((
                job,
                Err(RenderFailure::Worker("late worker failure".into())),
            ))
            .expect("late");
        state.advance(next).expect("discard stale");
        assert!(state.diagnostics.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn late_successful_embedded_result_cannot_replace_current_publication() {
        let (jobs, queued) = std::sync::mpsc::channel();
        let (completed, results) = std::sync::mpsc::channel();
        let mut state = MacosEmbeddedResourceState {
            cache: EmbeddedResourceCache::new(),
            jobs,
            results,
            control: RenderControl::default(),
            style: yu_assets::EmbeddedStyle::default(),
            theme: yu_core::ThemeId::Github,
            diagnostics: Vec::new(),
            publications: Vec::new(),
        };
        let range = TextRange::new(ByteOffset::ZERO, ByteOffset::new(3)).expect("range");
        let old =
            EmbeddedRenderRequest::new(Revision::INITIAL, range, EmbeddedResourceKind::Math, "x^2")
                .expect("old");
        let current_revision = Revision::new(1);
        let current =
            EmbeddedRenderRequest::new(current_revision, range, EmbeddedResourceKind::Math, "y^2")
                .expect("current");
        state
            .status_for(old, Revision::INITIAL)
            .expect("schedule old");
        let old_job = queued.try_recv().expect("old job");
        state
            .status_for(current.clone(), current_revision)
            .expect("schedule current");
        let current_job = queued.try_recv().expect("current job");
        let payload = |width| {
            yu_assets::EmbeddedRenderPayload::svg(
                width,
                20,
                format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"20\"/>"
                ),
            )
            .expect("payload")
        };
        completed
            .send((old_job.clone(), Ok(payload(99))))
            .expect("late old success");
        state.advance(current_revision).expect("discard old");
        assert!(state.publications.is_empty());
        assert!(state.diagnostics.is_empty());
        completed
            .send((current_job, Ok(payload(20))))
            .expect("current success");
        state.advance(current_revision).expect("accept current");
        assert_eq!(state.publications.len(), 1);
        let accepted = state.publications[0].clone();
        completed
            .send((old_job, Ok(payload(99))))
            .expect("old duplicate success");
        state.advance(current_revision).expect("discard duplicate");
        assert_eq!(state.publications.len(), 1);
        assert_eq!(state.publications[0].revision(), current_revision);
        assert_eq!(state.publications[0].payload(), accepted.payload());
        assert_eq!(
            state
                .status_for(current, current_revision)
                .expect("current cache"),
            YU_STORAGE_EMBEDDED_RESOURCE_READY
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_embedded_state_uses_native_math_and_mermaid_helper() {
        fn settled(state: &mut MacosEmbeddedResourceState, request: EmbeddedRenderRequest) -> u8 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let status = state
                    .status_for(request.clone(), Revision::INITIAL)
                    .expect("status");
                if status != YU_STORAGE_EMBEDDED_RESOURCE_PENDING {
                    return status;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "worker did not finish"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let mut state = MacosEmbeddedResourceState::new();
        let request = EmbeddedRenderRequest::new(
            Revision::INITIAL,
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(3)).expect("range"),
            EmbeddedResourceKind::Math,
            "x^2",
        )
        .expect("request");
        assert_eq!(
            settled(&mut state, request.clone()),
            YU_STORAGE_EMBEDDED_RESOURCE_READY
        );
        let publication = state
            .publication_for(request, Revision::INITIAL)
            .expect("publication")
            .expect("ready math publication");
        let yu_assets::EmbeddedRenderPayload::Svg {
            dimensions, markup, ..
        } = publication.payload()
        else {
            panic!("Math renderer must publish SVG");
        };
        let image = MacosEmbeddedSvgRasterizer::new()
            .rasterize(markup, dimensions.width(), dimensions.height())
            .expect("AppKit must rasterize the default Math SVG");
        assert_eq!(
            (image.width(), image.height()),
            (dimensions.width(), dimensions.height())
        );
        let mermaid = EmbeddedRenderRequest::new(
            Revision::INITIAL,
            TextRange::new(ByteOffset::new(4), ByteOffset::new(12)).expect("range"),
            EmbeddedResourceKind::Mermaid,
            "flowchart TD\nA-->B",
        )
        .expect("request");
        assert_eq!(
            settled(&mut state, mermaid.clone()),
            YU_STORAGE_EMBEDDED_RESOURCE_READY
        );
        let publication = state
            .publication_for(mermaid, Revision::INITIAL)
            .expect("query")
            .expect("ready diagram");
        let payload = publication.payload();
        let dimensions = payload.dimensions();
        let pixels = MacosEmbeddedSvgRasterizer::new()
            .rasterize(
                payload.markup().expect("svg"),
                dimensions.width(),
                dimensions.height(),
            )
            .expect("native diagram raster");
        assert_eq!(
            pixels.pixels()[3],
            0,
            "diagram canvas must be transparent on the native rasterizer"
        );
    }

    #[test]
    fn owned_byte_queries_support_null_zero_capacity_length_form() {
        let mut written = 0;
        assert_eq!(
            write_bytes("羽🙂".as_bytes(), ptr::null_mut(), 0, &mut written),
            YU_STORAGE_OK
        );
        assert_eq!(written, "羽🙂".len());
    }

    #[test]
    fn ffi_snapshot_queries_are_two_call_safe() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-query-{id}.md"));
        fs::write(&path, "# 羽\n日本語 🙂\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        let open_status =
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) };
        assert_eq!(open_status, YU_STORAGE_OK);
        assert!(!raw.is_null());

        let mut required = 0;
        assert_eq!(
            unsafe { yu_storage_session_copy_source(raw, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_OK
        );
        let mut source = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source(raw, source.as_mut_ptr(), source.len(), &mut written)
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, source.len());
        assert_eq!(
            String::from_utf8(source).expect("UTF-8 source"),
            "# 羽\n日本語 🙂\n"
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_selection_endpoints_preserve_visual_drag_direction() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-selection-endpoints-{id}.md"));
        let source = "alpha 日本語";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let end = source.encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    end,
                    0,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let mut endpoints = YuStorageSelectionEndpoints::default();
        assert_eq!(
            unsafe { yu_storage_session_selection_endpoints(raw, &mut endpoints) },
            YU_STORAGE_OK
        );
        assert_eq!(endpoints.revision, 0);
        assert_eq!(endpoints.anchor_utf16, end);
        assert_eq!(endpoints.focus_utf16, 0);
        assert_eq!(endpoints.affinity, YU_STORAGE_CARET_AFFINITY_UPSTREAM);

        // 有序区间由端点推导，不再单独跨 ABI。
        assert_eq!(endpoints.focus_utf16.min(endpoints.anchor_utf16), 0);
        assert_eq!(endpoints.focus_utf16.max(endpoints.anchor_utf16), end);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_set_selection_endpoints(raw, 0, end, 0, 0) },
            YU_STORAGE_STALE_REVISION
        );
        endpoints = YuStorageSelectionEndpoints {
            revision: 99,
            ..YuStorageSelectionEndpoints::default()
        };
        assert_eq!(
            unsafe { yu_storage_session_selection_endpoints(raw, &mut endpoints) },
            YU_STORAGE_OK
        );
        assert_eq!(endpoints.revision, 1);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_projection_is_source_backed_and_revision_bound() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-projection-{id}.md"));
        fs::write(&path, "**羽** [链接](https://example.com) 🙂\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        // 投影文本不再跨 ABI（不变量 I3）；直接查内部投影，断言不变。
        {
            // SAFETY: `raw` is a live session handle owned by this test.
            let session = unsafe { raw.as_mut() }.expect("session");
            let projection = session
                .session
                .visual_text_for_visual_state()
                .expect("projection");
            assert_eq!(projected_utf8(&projection), "羽 链接 🙂\n");
        }

        let mut caret = YuStorageProjectionCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_projection_caret(
                    raw,
                    0,
                    2,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut caret,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(caret.visual_utf16, 0);
        assert_eq!(caret.round_trip_source_utf16, 2);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"x".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_projection_caret(
                    raw,
                    0,
                    2,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut caret,
                )
            },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_projected_mirror_tracks_selection_bound_delimiter_reveal() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-reveal-{id}.md"));
        let source = "before **strong** after";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let strong = source.find("strong").expect("strong content");
        let source_utf16 = source[..strong + 2].encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    source_utf16,
                    source_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        {
            // SAFETY: `raw` is a live session handle owned by this test.
            let session = unsafe { raw.as_mut() }.expect("session");
            let projection = session
                .session
                .visual_text_for_visual_state()
                .expect("projection");
            assert_eq!(projected_utf8(&projection), source);
        }

        let end_utf16 = source.encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    end_utf16,
                    end_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        {
            // SAFETY: `raw` is a live session handle owned by this test.
            let session = unsafe { raw.as_mut() }.expect("session");
            let projection = session
                .session
                .visual_text_for_visual_state()
                .expect("projection");
            assert_eq!(projected_utf8(&projection), "before strong after");
        }

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_shaped_vertical_command_preserves_revision_and_selection_contract() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-shaped-vertical-{id}.md"));
        let source = "abcdefghij\nxy\n1234567890";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let first_line_end = source.find('\n').expect("line ending") as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    first_line_end,
                    first_line_end,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_move_vertical(
                    raw,
                    0,
                    YU_STORAGE_COMMAND_MOVE_DOWN,
                    14.0,
                    500.0,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(result.revision, 0);
        assert_eq!(result.selection_start_utf16, first_line_end + 1 + 2);
        assert_eq!(result.selection_end_utf16, first_line_end + 1 + 2);

        assert_eq!(
            unsafe {
                yu_storage_session_move_vertical(
                    raw,
                    0,
                    YU_STORAGE_COMMAND_MOVE_DOWN,
                    14.0,
                    500.0,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(result.revision, 0);
        let second_line_focus = result.selection_end_utf16;
        assert!(second_line_focus > first_line_end + 1 + 2);
        assert!(second_line_focus <= source.encode_utf16().count() as u64);

        let mut inserted = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut inserted) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_move_vertical(
                    raw,
                    0,
                    YU_STORAGE_COMMAND_MOVE_UP,
                    14.0,
                    500.0,
                    &mut result,
                )
            },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_shaped_projection_hit_test_is_revision_bound() {
        let id = temp_id();
        let path =
            std::env::temp_dir().join(format!("yu-storage-ffi-shaped-projection-hit-test-{id}.md"));
        let source = "before **粗体** after\nnext";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut hit = YuStorageProjectionHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_projection_hit_test(raw, 0, 0.0, 0.0, 14.0, 500.0, &mut hit)
            },
            YU_STORAGE_OK
        );
        assert_eq!(hit.revision, 0);
        assert_eq!(hit.source_utf16, 0);
        assert_eq!(hit.visual_utf16, 0);
        assert_eq!(hit.round_trip_source_utf16, 0);
        assert_eq!(hit.line, 0);
        assert!(hit.x.is_finite());
        assert!(hit.y.is_finite());

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_projection_hit_test(raw, 0, 0.0, 0.0, 14.0, 500.0, &mut hit)
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(hit.revision, 0);
        assert_eq!(hit.visual_utf16, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// 代码块内的点击吃内容原点：文档 y 进 hit_test 时减去 `content_origin_y`，
    /// 返回的 caret y 再加回来。判据取**同族参照**：点击同一行内文字的两个
    /// 不同高度（差一个行高减 5pt），映射必须落在同一 caret——source、line、
    /// 返回的 y 三者一致；漏掉原点的表现是偏下那个点越进行下沿落进块尾换行
    /// 的空行盒，不报错。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_projection_hit_test_includes_the_code_content_origin() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-code-hit-{id}.md"));
        let source = "```\nbody\n```\n\npara\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        // 光标放在段落里：代码块不被揭示，内容就是 "body\n" 两行盒
        // （文字行 + 块尾换行行盒）。
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    14,
                    14,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let (_, metrics, _) =
            core_text_layout(14.0, 500.0, yu_core::ThemeId::Github).expect("CoreText");
        let code_line = metrics.line_height() * yu_editor::layout_tokens::LINE_HEIGHT_CODE;
        // x 点在 'o' 上（内容内第二个字符）：行首的第一个字符带隐藏围栏的
        // 边界 bias，点它会映射到源码 0——那不是这一刀要测的东西。
        let probe = |y: f32| {
            let mut hit = YuStorageProjectionHit::default();
            assert_eq!(
                unsafe {
                    yu_storage_session_projection_hit_test(raw, 0, 18.0, y, 14.0, 500.0, &mut hit)
                },
                YU_STORAGE_OK
            );
            hit
        };
        let upper = probe(7.875 + 2.0);
        let lower = probe(7.875 + code_line - 3.0);
        assert_eq!(upper.source_utf16, 5, "靠上的点击落在 body 行内");
        assert_eq!(lower.source_utf16, 5, "贴行下沿的点击仍在 body 行内");
        assert_eq!(upper.line, 0);
        assert_eq!(lower.line, 0, "漏内容原点时这个点会落进块尾空行盒");
        // 同一 caret 的返回 y 一致，且含内容原点：块局部 caret 顶 0 + 原点 7.875。
        assert!(
            (upper.y - lower.y).abs() < 0.01,
            "同一 caret 的返回 y 必须一致：upper={} lower={}",
            upper.y,
            lower.y
        );
        assert!(
            (upper.y - 7.875).abs() < 0.01,
            "返回的 caret y 必须含内容原点（(8 + 1) × 14/16 = 7.875pt）：{}",
            upper.y
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_task_checkbox_hit_uses_current_published_frame_and_canonical_command() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-task-hit-{id}.md"));
        let source = "- [ ] todo\nparagraph\n- [x] done\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut frame = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut frame,
                )
            },
            YU_STORAGE_OK
        );
        let checkbox = {
            let state = unsafe { raw.as_ref() }.expect("session");
            state
                .macos_render_host
                .as_ref()
                .and_then(|host| host.builder.last_publication())
                .and_then(|publication| {
                    publication
                        .frame()
                        .scene()
                        .scene()
                        .primitives()
                        .iter()
                        .find_map(|primitive| match primitive {
                            Primitive::Ornament(task)
                                if task.role() == yu_scene::OrnamentRole::Border =>
                            {
                                Some(*task)
                            }
                            _ => None,
                        })
                })
                .expect("published task checkbox")
        };
        let bounds = checkbox.bounds();
        let point_x = bounds.x() + bounds.width() * 0.5;
        let point_y = bounds.y() + bounds.height() * 0.5;
        let mut hit = YuStorageTaskCheckboxHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_task_checkbox_hit_test(raw, 0, point_x, point_y, &mut hit)
            },
            YU_STORAGE_OK
        );
        assert_eq!(hit.revision, 0);
        assert_eq!(hit.block_index, 0);
        assert_eq!((hit.marker_start_utf16, hit.marker_end_utf16), (2, 5));
        assert_eq!(
            (hit.x, hit.y, hit.width, hit.height),
            (bounds.x(), bounds.y(), bounds.width(), bounds.height())
        );
        let hit_block = hit.block_index;

        let mut outside = YuStorageTaskCheckboxHit {
            revision: 99,
            ..YuStorageTaskCheckboxHit::default()
        };
        assert_eq!(
            unsafe {
                yu_storage_session_task_checkbox_hit_test(
                    raw,
                    0,
                    bounds.right() + 2.0,
                    point_y,
                    &mut outside,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(outside, YuStorageTaskCheckboxHit::default());

        assert_eq!(
            unsafe { yu_storage_session_begin_composition(raw, 0, 6, 6, b"n".as_ptr(), 1, 1, 1) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_task_checkbox_hit_test(raw, 0, point_x, point_y, &mut hit)
            },
            YU_STORAGE_INVALID_STATE
        );
        assert_eq!(
            unsafe { yu_storage_session_cancel_composition(raw, 0, 1) },
            YU_STORAGE_OK
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(
                    raw,
                    YU_STORAGE_COMMAND_TOGGLE_TASK,
                    hit_block,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert!(result.changed != 0);
        assert_eq!(result.revision, 1);
        assert!(
            unsafe { raw.as_ref() }
                .expect("session")
                .session
                .snapshot()
                .as_str()
                .starts_with("- [x] todo")
        );

        hit = YuStorageTaskCheckboxHit {
            revision: 99,
            ..YuStorageTaskCheckboxHit::default()
        };
        assert_eq!(
            unsafe {
                yu_storage_session_task_checkbox_hit_test(raw, 0, point_x, point_y, &mut hit)
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(hit, YuStorageTaskCheckboxHit::default());

        let mut undo = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(raw, YU_STORAGE_COMMAND_UNDO, 0, &mut undo)
            },
            YU_STORAGE_OK
        );
        assert!(undo.changed != 0);
        assert_eq!(undo.revision, 2);
        assert!(
            unsafe { raw.as_ref() }
                .expect("session")
                .session
                .snapshot()
                .as_str()
                .starts_with("- [ ] todo")
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_nested_task_click_undo_redo_save_and_reopen_preserves_source() {
        for prefix in ["- > - [ ] ", "> 3. > - [ ] ", "3. > 7. > - [ ] "] {
            let source = format!("{prefix}todo 中文\n\nend\n");
            let marker = source.find("[ ]").expect("checkbox") as u64;
            for appearance in [YU_STORAGE_APPEARANCE_LIGHT, YU_STORAGE_APPEARANCE_DARK] {
                let path = std::env::temp_dir().join(format!("yu-nested-task-{}.md", temp_id()));
                fs::write(&path, &source).expect("fixture");
                let bytes = path.to_string_lossy().as_bytes().to_vec();
                let mut raw = ptr::null_mut();
                assert_eq!(
                    unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
                    YU_STORAGE_OK
                );
                let mut frame = YuStorageMacosRenderHostSnapshot::default();
                assert_eq!(
                    unsafe {
                        yu_storage_session_macos_render_host_frame(
                            raw, 0, 16.0, 600.0, 0.0, 800.0, 0, appearance, &mut frame,
                        )
                    },
                    YU_STORAGE_OK
                );
                let bounds = unsafe { raw.as_ref() }
                    .expect("session")
                    .macos_render_host
                    .as_ref()
                    .and_then(|host| host.builder.last_publication())
                    .expect("publication")
                    .frame()
                    .scene()
                    .scene()
                    .primitives()
                    .iter()
                    .find_map(|primitive| match primitive {
                        Primitive::Ornament(task)
                            if task.role() == yu_scene::OrnamentRole::Border
                                && task.source().start().get() == marker
                                && task.source().end().get() == marker + 3 =>
                        {
                            Some(task.bounds())
                        }
                        _ => None,
                    })
                    .expect("painted checkbox");
                let mut hit = YuStorageTaskCheckboxHit::default();
                assert_eq!(
                    unsafe {
                        yu_storage_session_task_checkbox_hit_test(
                            raw,
                            0,
                            bounds.x() + bounds.width() * 0.5,
                            bounds.y() + bounds.height() * 0.5,
                            &mut hit,
                        )
                    },
                    YU_STORAGE_OK
                );
                assert_eq!(
                    (hit.marker_start_utf16, hit.marker_end_utf16),
                    (marker, marker + 3)
                );
                for (command, expected) in [
                    (
                        YU_STORAGE_COMMAND_TOGGLE_TASK,
                        source.replacen("[ ]", "[x]", 1),
                    ),
                    (YU_STORAGE_COMMAND_UNDO, source.clone()),
                    (YU_STORAGE_COMMAND_REDO, source.replacen("[ ]", "[x]", 1)),
                ] {
                    let mut result = YuStorageCommandResult::default();
                    assert_eq!(
                        unsafe {
                            yu_storage_session_execute_command(
                                raw,
                                command,
                                hit.block_index,
                                &mut result,
                            )
                        },
                        YU_STORAGE_OK
                    );
                    assert_ne!(result.changed, 0);
                    assert_eq!(
                        unsafe { raw.as_ref() }
                            .expect("session")
                            .session
                            .snapshot()
                            .as_str(),
                        expected
                    );
                }
                let mut revision = 0;
                let mut written = 0;
                let mut changed = 0;
                assert_eq!(
                    unsafe {
                        yu_storage_session_save(raw, &mut revision, &mut written, &mut changed)
                    },
                    YU_STORAGE_OK
                );
                let expected = source.replacen("[ ]", "[x]", 1);
                assert_eq!(fs::read_to_string(&path).expect("saved"), expected);
                unsafe { yu_storage_session_destroy(raw) };
                raw = ptr::null_mut();
                assert_eq!(
                    unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
                    YU_STORAGE_OK
                );
                assert_eq!(
                    unsafe { raw.as_ref() }
                        .expect("reopened")
                        .session
                        .snapshot()
                        .as_str(),
                    expected
                );
                unsafe { yu_storage_session_destroy(raw) };
                fs::remove_file(path).expect("cleanup");
            }
        }
    }

    /// 复数选区在 FFI 边界上的往返：两遍协议、归一化、primary 的下标。
    ///
    /// **归一化在 Rust 侧一家做。** 平台送来逆序、重叠、同一偏移两次的输入
    /// （⌥ 点在一段选区里就是最后这种），这里不拒绝，收敛掉。让平台先自己排
    /// 一遍就是第二份合并实现，而两份合并必定分叉。
    #[test]
    fn ffi_selections_round_trip_and_normalize() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-selections-{id}.md"));
        fs::write(&path, "alpha beta gamma\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let endpoint = |anchor: u64, focus: u64| YuStorageSelectionEndpoints {
            revision: 0,
            anchor_utf16: anchor,
            focus_utf16: focus,
            affinity: YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
        };
        let read = |raw: *mut YuStorageSession| -> (Vec<(u64, u64)>, usize) {
            let mut written = 0;
            let mut primary = usize::MAX;
            // 第一遍：只问条数。
            assert_eq!(
                unsafe {
                    yu_storage_session_selections(
                        raw,
                        0,
                        ptr::null_mut(),
                        0,
                        &mut written,
                        &mut primary,
                    )
                },
                YU_STORAGE_OK
            );
            let mut buffer = vec![YuStorageSelectionEndpoints::default(); written];
            assert_eq!(
                unsafe {
                    yu_storage_session_selections(
                        raw,
                        0,
                        buffer.as_mut_ptr(),
                        buffer.len(),
                        &mut written,
                        &mut primary,
                    )
                },
                YU_STORAGE_OK
            );
            (
                buffer
                    .iter()
                    .map(|entry| (entry.anchor_utf16, entry.focus_utf16))
                    .collect(),
                primary,
            )
        };

        // 开门就是一条选区——`Selections` 永远至少有一条。
        assert_eq!(read(raw), (vec![(0, 0)], 0));

        // 逆序 + 重叠 + 一个停在选区边界上的空光标，一起送进去。
        let messy = [
            endpoint(11, 16),
            endpoint(6, 6),
            endpoint(2, 0),
            endpoint(0, 4),
        ];
        assert_eq!(
            unsafe { yu_storage_session_set_selections(raw, 0, messy.as_ptr(), messy.len(), 1) },
            YU_STORAGE_OK
        );
        let (ranges, primary) = read(raw);
        assert_eq!(
            ranges,
            // 端点保留方向，所以并出来的那一条是 anchor=4、focus=0：两条都不是
            // primary 时方向归靠前的那一条，而它（输入的 `endpoint(2, 0)`）是
            // 反向的。这条规则本身值得断言——丢掉方向的表现是拖选到一半松手，
            // 选区突然翻个个儿。
            vec![(4, 0), (6, 6), (11, 16)],
            "`0..2` 与 `0..4` 要并成一条；`6` 那个光标不与任何一条相接"
        );
        assert_eq!(primary, 1, "primary 是输入里的 `6`，排序之后在中间");

        // 数组小了要明确拒绝，不能拷半份出去。
        let mut short = vec![YuStorageSelectionEndpoints::default(); 2];
        let mut written = 0;
        let mut primary_out = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_selections(
                    raw,
                    0,
                    short.as_mut_ptr(),
                    short.len(),
                    &mut written,
                    &mut primary_out,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );

        // 一条都不给是错误：「一个光标都没有」在这个模型里不存在。
        assert_eq!(
            unsafe { yu_storage_session_set_selections(raw, 0, messy.as_ptr(), 0, 0) },
            YU_STORAGE_INVALID_SELECTION
        );
        // primary 越界同样是错误。
        assert_eq!(
            unsafe {
                yu_storage_session_set_selections(raw, 0, messy.as_ptr(), messy.len(), messy.len())
            },
            YU_STORAGE_INVALID_SELECTION
        );
        // Revision 失配不得触碰选区。
        assert_eq!(
            unsafe { yu_storage_session_set_selections(raw, 99, messy.as_ptr(), messy.len(), 0) },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(read(raw).0, vec![(4, 0), (6, 6), (11, 16)]);

        // **单数入口给的是 primary。** 两条路各自回答自己的问题。
        let mut single = YuStorageSelectionEndpoints::default();
        assert_eq!(
            unsafe { yu_storage_session_selection_endpoints(raw, &mut single) },
            YU_STORAGE_OK
        );
        assert_eq!((single.anchor_utf16, single.focus_utf16), (6, 6));

        unsafe { yu_storage_session_destroy(raw) };
        let _ = fs::remove_file(&path);
    }

    /// 认不得的外观字节按浅色画，**不拒整帧**。
    ///
    /// 这条是反向验证时活下来的一个变异逼出来的：把 `_ => Light` 改成
    /// `_ => Dark` 之后全套用例照样绿。没有它，未来系统多一种外观时的行为
    /// 由一行没人读过的 `match` 决定——而两种可能的错法差别很大：拒整帧是
    /// 一片空白（违反 I5「永不白屏」），认成深色是白底上的浅字。
    #[cfg(target_os = "macos")]
    #[test]
    fn an_unknown_appearance_byte_falls_back_to_light() {
        assert_eq!(
            appearance_from_raw(YU_STORAGE_APPEARANCE_LIGHT),
            Appearance::Light
        );
        assert_eq!(
            appearance_from_raw(YU_STORAGE_APPEARANCE_DARK),
            Appearance::Dark
        );
        assert_eq!(
            appearance_from_raw(YU_STORAGE_THEME_YU_LIGHT),
            Appearance::YuLight
        );
        assert_eq!(
            appearance_from_raw(YU_STORAGE_THEME_YU_DARK),
            Appearance::YuDark
        );
        for raw in [4u8, 7, 255] {
            assert_eq!(
                appearance_from_raw(raw),
                Appearance::Light,
                "认不得的外观字节 {raw} 必须按浅色画，而不是拒整帧或猜成深色"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_render_host_config_tracks_document_scroll_origin() {
        let viewport = ViewportSpan::new(137.5, 240.0);
        let config = macos_render_host_config(viewport, 14.0, 500.0, 240.0, 2.0, Appearance::Light)
            .expect("valid macOS render host config");

        assert_eq!(config.viewport().scroll_y(), 137.5);
        assert_eq!(config.viewport().height(), 240.0);
        assert_eq!(config.scene_viewport().x(), 0.0);
        assert_eq!(config.scene_viewport().y(), 137.5);
        assert_eq!(config.scene_viewport().width(), 500.0);
        assert_eq!(config.scene_viewport().height(), 240.0);
        // backing scale 必须进入配置：字形按它取样，后端再除回逻辑坐标。
        assert_eq!(config.raster_scale(), 2.0);
    }

    /// 没有已提交的帧时，任何几何都不能被判为"当前"。
    ///
    /// 这条判断决定平台是否跳过提交。误判为当前会让编辑或滚动后的内容停留在
    /// 上一帧；而在 surface 尚未建立时误判为当前，则会让窗口一直空白。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_frame_is_current_requires_a_submitted_frame() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-frame-current-{id}.md"));
        fs::write(&path, "# \u{6807}\u{9898}\n\nbody\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let geometry = YuStorageFrameGeometry {
            size: 14.0,
            max_width: 500.0,
            scroll_y: 0.0,
            viewport_height: 240.0,
            surface_width: 500.0,
            surface_height: 240.0,
            scale: 2.0,
        };
        let mut current = 1_u8;
        assert_eq!(
            unsafe {
                yu_storage_session_frame_is_current(
                    raw,
                    &geometry,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut current,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(current, 0, "尚未提交任何帧时不得判为当前");
        let mut presented_at = 123.0;
        assert_eq!(
            unsafe {
                yu_storage_session_frame_presentation_time(
                    raw,
                    &geometry,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut presented_at,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            presented_at, 0.0,
            "No submission must not expose a timestamp"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_frame_presentation_time(
                    raw,
                    &geometry,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    ptr::null_mut(),
                )
            },
            YU_STORAGE_NULL_POINTER
        );

        // 非法几何必须被拒绝，而不是当作"不同"从而每帧重画。
        for invalid in [
            YuStorageFrameGeometry {
                size: 0.0,
                ..geometry
            },
            YuStorageFrameGeometry {
                scale: f64::NAN,
                ..geometry
            },
            YuStorageFrameGeometry {
                viewport_height: -1.0,
                ..geometry
            },
        ] {
            let mut out = 1_u8;
            assert_eq!(
                unsafe {
                    yu_storage_session_frame_is_current(
                        raw,
                        &invalid,
                        YU_STORAGE_APPEARANCE_LIGHT,
                        &mut out,
                    )
                },
                YU_STORAGE_EDITOR_ERROR
            );
            assert_eq!(out, 0);
            let mut presented_at = 123.0;
            assert_eq!(
                unsafe {
                    yu_storage_session_frame_presentation_time(
                        raw,
                        &invalid,
                        YU_STORAGE_APPEARANCE_LIGHT,
                        &mut presented_at,
                    )
                },
                YU_STORAGE_EDITOR_ERROR
            );
            assert_eq!(presented_at, 0.0);
        }

        // 空指针必须返回明确状态，不得写入半成品输出。
        let mut out = 1_u8;
        assert_eq!(
            unsafe {
                yu_storage_session_frame_is_current(
                    raw,
                    std::ptr::null(),
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut out,
                )
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(out, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// 测试用：不经 CoreText，在指定 block 上开始一次分隔线拖动。
    ///
    /// 产品走的是 `macos_table_resize_begin_at_point`（文档坐标 + CoreText 排版）。
    /// 度量版此前也有一个 FFI 入口，但产品从不调用——它只有测试需要，因此现在
    /// 留在测试里，不再穿过 ABI（不变量 I3）。
    fn begin_table_resize_for_test(
        raw: *mut YuStorageSession,
        block_index: usize,
        point_x: f32,
        point_y: f32,
        tolerance: f32,
        pointer_position: f32,
    ) -> Result<YuStorageTableResizeHit, i32> {
        // SAFETY: `raw` is a live session handle owned by the calling test.
        let session = unsafe { raw.as_mut() }.expect("session");
        let config = LayoutConfig::new(20.0, 2.0).with_default_advance(1.0);
        let layout = session
            .session
            .block_layout(block_index, config)
            .map_err(storage_status)?;
        let table = layout.table().ok_or(YU_STORAGE_INVALID_SELECTION)?;
        let hit = match table.resize_hit_test(LayoutPoint::new(point_x, point_y), tolerance) {
            Ok(Some(hit)) => hit,
            Ok(None) | Err(_) => return Err(YU_STORAGE_INVALID_SELECTION),
        };
        begin_table_resize_session(session, block_index, hit, pointer_position)
    }

    /// 帧身份必须覆盖每一项「不推进 Revision 却改变画面」的状态。
    ///
    /// 这个判断决定平台是否跳过提交，而漏掉一项不会报错——只会让光标停在原处、
    /// preedit 不更新、拖动中的列宽不动。三者都表现为「编辑器卡住了」，却没有
    /// 任何日志或错误码可查，正是本项目最危险的失败模式。
    ///
    /// 反向验证：把 `selection` 从 `FrameKey` 去掉，第二段断言失败；
    /// 把 `table_resize` 去掉，第三段断言失败。
    /// 未知的剪贴板格式必须被拒绝，且不得留下长度。
    ///
    /// 纯文本与 HTML 合成一个入口后，传错 format 会静默地把另一种表示放进剪贴板
    /// ——粘贴出来是 HTML 标签或反过来，没有任何报错。
    #[test]
    fn ffi_typed_table_clipboard_converts_both_directions() {
        for html in [false, true] {
            let original = if html {
                "<table><tr><td>KEEP</td></tr></table>\r\n"
            } else {
                "| KEEP |\n| --- |\n"
            };
            let path =
                std::env::temp_dir().join(format!("yu-typed-clipboard-{}-{html}.md", temp_id()));
            fs::write(&path, original).expect("fixture");
            let bytes = path.to_string_lossy().as_bytes().to_vec();
            let mut raw = ptr::null_mut();
            assert_eq!(
                unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
                YU_STORAGE_OK
            );
            let at = original.find("KEEP").expect("cell") as u64;
            assert_eq!(
                unsafe {
                    yu_storage_session_set_selection_endpoints(
                        raw,
                        0,
                        at,
                        at,
                        YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    )
                },
                YU_STORAGE_OK
            );
            let cell = if html {
                "**中文 bold** [link](https://example.com)"
            } else {
                "<p><b>中文 bold</b> <a href='https://example.com'>link</a></p>"
            };
            if !html {
                yu_export::import_html_fragment(cell).expect("HTML import");
            }
            let mut result = YuStorageCommandResult::default();
            let format = if html {
                YU_STORAGE_FRAGMENT_MARKDOWN
            } else {
                YU_STORAGE_FRAGMENT_HTML
            };
            assert_eq!(
                unsafe {
                    yu_storage_session_paste_fragments(
                        raw,
                        0,
                        cell.as_ptr(),
                        cell.len(),
                        [cell.len()].as_ptr(),
                        1,
                        1,
                        format,
                        &mut result,
                    )
                },
                YU_STORAGE_OK,
                "destination HTML={html}"
            );
            let actual = unsafe { &*raw }.session.snapshot();
            assert!(
                actual.as_str().contains(if html {
                    "<strong>中文 bold</strong>"
                } else {
                    "**中文 bold**"
                }),
                "{}",
                actual.as_str()
            );
            unsafe {
                yu_storage_session_destroy(raw);
            }
            fs::remove_file(path).expect("cleanup");
        }
    }

    #[test]
    fn ffi_copy_selection_rejects_unknown_formats() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-clipboard-format-{id}.md"));
        fs::write(&path, "**\u{7fbd}** plain\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let end = "**\u{7fbd}** plain\n".encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    0,
                    end,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        let mut text_len = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
                    YU_STORAGE_CLIPBOARD_TEXT,
                    ptr::null_mut(),
                    0,
                    &mut text_len,
                )
            },
            YU_STORAGE_OK
        );
        let mut html_len = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
                    YU_STORAGE_CLIPBOARD_HTML,
                    ptr::null_mut(),
                    0,
                    &mut html_len,
                )
            },
            YU_STORAGE_OK
        );
        assert!(text_len > 0 && html_len > text_len, "两种表示必须不同");

        for unknown in [4_u8, 255] {
            let mut written = 99;
            assert_eq!(
                unsafe {
                    yu_storage_session_copy_selection(
                        raw,
                        0,
                        unknown,
                        ptr::null_mut(),
                        0,
                        &mut written,
                    )
                },
                YU_STORAGE_INVALID_COMMAND
            );
            assert_eq!(written, 0, "被拒绝的格式不得留下长度");
        }

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// PROBE 不得改变状态，未知 action 不得被当成 BEGIN 执行。
    ///
    /// 探测与开始拖动合成一个入口之后，「hover 只读」这条约束从两份实现各自
    /// 保证变成了一处 action 分支。分错会在鼠标划过表格时静默开出一个手势，
    /// 后续的 update 就会真的改列宽——没有报错。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_table_resize_probe_does_not_open_a_gesture() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-resize-probe-{id}.md"));
        fs::write(&path, "| A | B |\n| --- | :---: |\n| 1 | 2 |\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        // Use the published divider geometry, including the first figure's
        // margin; y=1 belongs to the reading-column whitespace, not the table.
        let mut dividers = [YuStorageTableResizeAccessibilityDivider::default(); 8];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_accessibility_dividers(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    dividers.as_mut_ptr(),
                    dividers.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        let geometry = dividers[..written]
            .iter()
            .find(|d| d.kind == YU_STORAGE_TABLE_RESIZE_COLUMN)
            .expect("column geometry");
        let divider = geometry.x;
        let point_y = geometry.y + geometry.height * 0.5;
        let mut empty_hover = 9;
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_hover(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    divider,
                    1.0,
                    0.4,
                    &mut empty_hover,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(empty_hover, 0, "figure top margin must not resize a column");
        let mut probe = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_PROBE,
                    14.0,
                    500.0,
                    divider,
                    point_y,
                    0.4,
                    0.0,
                    &mut probe,
                )
            },
            YU_STORAGE_OK
        );

        assert_eq!(probe.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);

        // 探测不得开出手势：紧接着的 UPDATE 必须报「没有手势」。
        let mut commit = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    divider + 1.0,
                    &mut commit,
                )
            },
            YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE
        );

        // 未知 action 必须被拒绝，同样不得开出手势。
        let mut rejected = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_at_point(
                    raw,
                    0,
                    9,
                    14.0,
                    500.0,
                    divider,
                    point_y,
                    0.4,
                    divider,
                    &mut rejected,
                )
            },
            YU_STORAGE_INVALID_COMMAND
        );
        assert_eq!(rejected, YuStorageTableResizeHit::default());
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    divider + 1.0,
                    &mut commit,
                )
            },
            YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE
        );

        // BEGIN 之后 UPDATE 必须成立。
        let mut begun = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_BEGIN,
                    14.0,
                    500.0,
                    divider,
                    point_y,
                    0.4,
                    divider,
                    &mut begun,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    divider + 1.0,
                    &mut commit,
                )
            },
            YU_STORAGE_OK
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_frame_key_notices_state_that_does_not_advance_revision() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-frame-key-{id}.md"));
        fs::write(&path, "| A | B |\n| --- | :---: |\n| 1 | 2 |\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let request = YuStorageFrameGeometry {
            size: 14.0,
            max_width: 500.0,
            scroll_y: 0.0,
            viewport_height: 240.0,
            surface_width: 500.0,
            surface_height: 240.0,
            scale: 2.0,
        };
        let geometry = frame_geometry(&request).expect("几何合法");
        let capture = |geometry: FrameGeometry| {
            // SAFETY: `raw` is a live session handle and no other borrow is
            // outstanding at this point.
            let session = unsafe { raw.as_ref() }.expect("session");
            frame_key(session, Appearance::Light, geometry)
        };

        let baseline = capture(geometry);
        assert_eq!(
            baseline,
            capture(geometry),
            "状态未变时必须得到同一身份，否则每帧都会重画"
        );

        // 光标移动不推进 Revision，但会改变 caret 装饰。
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    2,
                    2,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let moved = capture(geometry);
        assert_eq!(
            moved.revision(),
            baseline.revision(),
            "选区变化不推进 Revision"
        );
        assert_ne!(baseline, moved, "光标移动必须让帧身份改变");

        // **加一根光标同样不推进 Revision、不改几何。**
        //
        // 帧身份里的选区从「一个」变成「一组」之后，最容易漏的就是**条数**：
        // 只比 primary 的话，从一根光标变成三根会被判为等价——按下「选中全部
        // 匹配」，画面一动不动，不报错、不 panic。这一条与上面那条光标移动
        // 是两回事：那条改的是位置，这条改的是根数。
        let two_carets = [
            YuStorageSelectionEndpoints {
                revision: 0,
                anchor_utf16: 2,
                focus_utf16: 2,
                affinity: YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
            YuStorageSelectionEndpoints {
                revision: 0,
                anchor_utf16: 5,
                focus_utf16: 5,
                affinity: YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        ];
        assert_eq!(
            unsafe {
                yu_storage_session_set_selections(raw, 0, two_carets.as_ptr(), two_carets.len(), 0)
            },
            YU_STORAGE_OK
        );
        let two = capture(geometry);
        assert_eq!(
            two.revision(),
            moved.revision(),
            "加一根光标不推进 Revision"
        );
        assert_eq!(two.geometry(), moved.geometry(), "加一根光标不改变几何");
        assert_eq!(
            two.selections()[0],
            moved.selections()[0],
            "primary 那一条没有动——只比它的话这一步会被判为等价"
        );
        assert_ne!(moved, two, "光标根数变化必须让帧身份改变");

        // 收回一根同样是一次变化。
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    2,
                    2,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let back_to_one = capture(geometry);
        assert_ne!(two, back_to_one, "收回一根光标必须让帧身份改变");
        assert_eq!(back_to_one, moved, "回到同一个状态必须给出同一个身份");

        // 换查询同样不推进 Revision、不改几何、不改选区。少了这一项，在搜索
        // 框里打字画面会一动不动——不报错、不 panic。
        let query = b"a";
        assert_eq!(
            unsafe { yu_storage_session_set_search_query(raw, query.as_ptr(), query.len()) },
            YU_STORAGE_OK
        );
        let searched = capture(geometry);
        assert_eq!(
            searched.revision(),
            moved.revision(),
            "换查询不推进 Revision"
        );
        assert_eq!(
            searched.selections(),
            moved.selections(),
            "换查询不改变选区"
        );
        assert_eq!(searched.geometry(), moved.geometry(), "换查询不改变几何");
        assert_ne!(moved, searched, "换查询必须让帧身份改变");

        // 收掉搜索也是一次变化：高亮要消失。
        assert_eq!(
            unsafe { yu_storage_session_set_search_query(raw, ptr::null(), 0) },
            YU_STORAGE_OK
        );
        assert_ne!(searched, capture(geometry), "收掉搜索必须让帧身份改变");

        // 拖动列分隔线既不推进 Revision 也不改变几何。
        let hit =
            begin_table_resize_for_test(raw, 0, 10.1, 0.5, 0.2, 3.1).expect("begin table resize");
        assert_eq!(hit.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        let mut preview = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    4.1,
                    &mut preview,
                )
            },
            YU_STORAGE_OK
        );
        let dragged = capture(geometry);
        assert_eq!(dragged.revision(), moved.revision(), "拖动不推进 Revision");
        assert_eq!(dragged.geometry(), moved.geometry(), "拖动不改变平台几何");
        assert_ne!(moved, dragged, "列宽覆盖变化必须让帧身份改变");

        // 再拖一格：同一个 gesture 内的位移同样必须被看见。
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    5.1,
                    &mut preview,
                )
            },
            YU_STORAGE_OK
        );
        assert_ne!(dragged, capture(geometry), "同一手势内的位移必须被看见");

        // 几何本身仍然参与比较。
        let scrolled = frame_geometry(&YuStorageFrameGeometry {
            scroll_y: 40.0,
            ..request
        })
        .expect("几何合法");
        assert_ne!(capture(geometry), capture(scrolled), "滚动必须让帧身份改变");

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_table_resize_finish_preserves_widths_after_edit() {
        let path = std::env::temp_dir().join(format!("yu-confirmed-widths-{}.md", temp_id()));
        let source = "| A | B |\n| --- | --- |\n| value | other |\n";
        fs::write(&path, source).expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        // The native host normally supplies the same layout policy at BEGIN.
        let config = LayoutConfig::new(500.0, 16.0).with_default_advance(8.0);
        let session = unsafe { raw.as_mut() }.expect("live session");
        let editor = session.session.document_mut().editor_mut();
        editor
            .set_viewport_config(yu_editor::ViewportConfig::new(config, 16.0, 0.0))
            .expect("viewport");
        let layout = editor
            .block_layout_for_visual_state(0, config)
            .expect("layout");
        let table = layout.table().expect("table");
        let x = table.bounds().x() + table.column_widths()[0];
        let hit = table
            .resize_hit_test(LayoutPoint::new(x, table.bounds().y() + 1.0), 1.0)
            .expect("hit")
            .expect("divider");
        begin_table_resize_session(session, 0, hit, x).expect("begin");
        let mut result = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    x - 60.0,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_FINISH,
                    0.0,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        let session = unsafe { raw.as_mut() }.expect("live session");
        assert!(session.table_resize_override.is_none());
        let editor = session.session.document_mut().editor_mut();
        assert_eq!(editor.table_width_generation(), 1);
        let before = editor
            .block_layout_for_visual_state(0, config)
            .expect("confirmed")
            .table()
            .expect("table")
            .column_widths()
            .to_vec();
        editor
            .execute(yu_editor::EditorCommand::SelectTableCells {
                anchor: ByteOffset::new(source.find("value").expect("cell") as u64),
                focus: ByteOffset::new(source.find("value").expect("cell") as u64),
            })
            .expect("select");
        editor
            .execute(yu_editor::EditorCommand::PasteTsv(
                "longer cell text".into(),
            ))
            .expect("edit");
        let layout = editor
            .block_layout_for_visual_state(0, config)
            .expect("after edit");
        for (a, b) in layout
            .table()
            .expect("table")
            .column_widths()
            .iter()
            .zip(&before)
        {
            assert!((a - b).abs() < 0.001);
        }
        assert_eq!(editor.table_width_generation(), 1);
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// 未知的 action 必须被拒绝，且不得触碰手势状态。
    ///
    /// 三个动作合成一个入口之后，「传错 action 会发生什么」成了一个新的失败面。
    /// 静默地把未知值当成某个动作执行，会在拖动中途清掉覆盖或提交错误几何——
    /// 都不报错。
    #[test]
    fn ffi_table_resize_action_rejects_unknown_actions() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-resize-action-{id}.md"));
        fs::write(&path, "| A | B |\n| --- | :---: |\n| 1 | 2 |\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let hit =
            begin_table_resize_for_test(raw, 0, 10.1, 0.5, 0.2, 3.1).expect("begin table resize");
        assert_eq!(hit.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        let mut preview = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    4.1,
                    &mut preview,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(preview.final_position, 11.0);

        for unknown in [4_u8, 255] {
            let mut out = preview;
            assert_eq!(
                unsafe { yu_storage_session_table_resize_action(raw, 0, unknown, 9.9, &mut out) },
                YU_STORAGE_INVALID_COMMAND
            );
            assert_eq!(
                out,
                YuStorageTableResizeCommit::default(),
                "不得留下半成品输出"
            );
        }

        // 手势必须仍然活着：未知 action 不得当成 cancel 或 finish。
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    5.1,
                    &mut preview,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(preview.final_position, 12.0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_table_resize_gesture_lifecycle_is_revision_bound_and_source_neutral() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-table-gesture-{id}.md"));
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let hit =
            begin_table_resize_for_test(raw, 0, 10.1, 0.5, 0.2, 3.1).expect("begin table resize");
        assert_eq!(hit.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert_eq!(hit.index, 0);
        assert_eq!(hit.position, 10.0);

        // 已有手势时不得再开一个。
        assert_eq!(
            begin_table_resize_for_test(raw, 0, 10.1, 0.5, 0.2, 3.1),
            Err(YU_STORAGE_INVALID_STATE)
        );

        let mut preview = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    4.1,
                    &mut preview,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(preview.revision, 0);
        assert_eq!(preview.block_index, 0);
        assert_eq!(preview.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert_eq!(preview.index, 0);
        assert_eq!(preview.initial_position, 10.0);
        assert_eq!(preview.final_position, 11.0);
        assert_eq!(preview.delta, 1.0);

        let mut committed = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_FINISH,
                    0.0,
                    &mut committed,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(committed, preview);
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    4.1,
                    &mut preview,
                )
            },
            YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE
        );
        assert_eq!(preview, YuStorageTableResizeCommit::default());
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_CANCEL,
                    0.0,
                    &mut YuStorageTableResizeCommit::default(),
                )
            },
            YU_STORAGE_OK
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"x".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_FINISH,
                    0.0,
                    &mut committed,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(committed, YuStorageTableResizeCommit::default());
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    1,
                    YU_STORAGE_TABLE_RESIZE_CANCEL,
                    0.0,
                    &mut YuStorageTableResizeCommit::default(),
                )
            },
            YU_STORAGE_OK
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn document_resource_geometry_tracks_sizes_not_bitmap_pixels() {
        let revision = Revision::new(7);
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(10)).expect("range");
        let mut cache = yu_assets::ImageCache::new();
        let mut publish = |width, byte| {
            cache
                .publish_decoded(
                    yu_assets::ImageRequest::new(revision, source, "image.png").expect("request"),
                    revision,
                    yu_assets::DecodedImage::new(width, 3, vec![byte; width as usize * 3 * 4])
                        .expect("pixels"),
                )
                .expect("publication")
        };
        let first = publish(2, 255);
        let key = first.key().fingerprint();
        let mut resources = MacosImageResourceState::new().expect("resources");
        assert_eq!(resources.geometry_version(revision), 0);
        resources
            .intrinsics
            .insert(key, first.intrinsic_publication());
        let intrinsic_version = resources.geometry_version(revision);
        assert_ne!(intrinsic_version, 0);
        resources.publications.insert(key, publish(2, 0));
        assert_eq!(
            resources.geometry_version(revision),
            intrinsic_version,
            "pixel replacement with unchanged size is decoration-only"
        );
        let resized = publish(4, 255);
        resources.publications.insert(key, resized.clone());
        let resized_version = resources.geometry_version(revision);
        assert_ne!(resized_version, intrinsic_version);
        resources
            .intrinsics
            .insert(key, resized.intrinsic_publication());
        resources.publications.clear();
        assert_eq!(
            resources.geometry_version(revision),
            resized_version,
            "bitmap eviction preserves known dimensions"
        );
        assert_eq!(
            resources.geometry_version(Revision::new(8)),
            0,
            "stale revisions must not affect current geometry"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_list_markers_have_geometry_and_preserve_source_carets() {
        let source =
            "# Heading\n\n- first\n  - child\n    - deep\n\n> - quoted\n\n1. ordered\n\nend\n";
        for theme in [yu_core::ThemeId::Github, yu_core::ThemeId::Night] {
            for zoom in [0.75, 1.0, 1.5] {
                let (shaper, _, config) =
                    core_text_layout(16.0 * zoom, 500.0, theme).expect("native font");
                let mut document = yu_editor::EditorDocument::new(source);
                document
                    .set_viewport_config(yu_editor::ViewportConfig::new(config, 16.0 * zoom, 0.0))
                    .expect("config");
                let snapshot = document
                    .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                    .expect("snapshot");
                let mut kinds = Vec::new();
                let mut ordered = 0;
                for block in snapshot.blocks() {
                    let layout = block.layout();
                    let Some(marker) = layout.ornaments().marker() else {
                        continue;
                    };
                    if let Some(shape) = marker.shape() {
                        kinds.push(shape.kind);
                        let bounds = layout.marker_bounds().expect("geometry").expect("shape");
                        assert!(bounds.width() >= 3.0 * zoom);
                        assert_eq!(bounds.width(), bounds.height());
                        assert!(
                            layout
                                .glyphs()
                                .iter()
                                .all(|glyph| glyph.source() != marker.source()),
                            "a geometric marker must not leave a second bullet glyph"
                        );
                        let first = layout
                            .clusters()
                            .iter()
                            .find(|cluster| !cluster.visual().is_empty())
                            .expect("body");
                        let caret = layout
                            .caret_for_source(first.source().start(), Bias::After)
                            .expect("body caret");
                        assert!(
                            bounds.x() + bounds.width() < caret.point().x(),
                            "marker stays in gutter"
                        );
                        // Hidden Markdown prefixes share the first visual boundary;
                        // an interior text boundary has one exact source position.
                        let interior = layout
                            .caret_for_source(first.source().end(), Bias::After)
                            .expect("interior caret");
                        assert_eq!(
                            layout.hit_test(interior.point()).expect("hit").source(),
                            interior.source()
                        );
                    } else {
                        ordered += 1;
                        assert!(marker.shaped().is_some(), "numbered marker remains text");
                    }
                }
                assert_eq!(ordered, 1);
                assert_eq!(kinds.len(), 4);
                if theme == yu_core::ThemeId::Github {
                    assert!(kinds.contains(&yu_editor::MarkerShapeKind::Disc));
                    assert!(kinds.contains(&yu_editor::MarkerShapeKind::Circle));
                    assert!(kinds.contains(&yu_editor::MarkerShapeKind::Square));
                } else {
                    assert!(
                        kinds
                            .iter()
                            .all(|kind| *kind == yu_editor::MarkerShapeKind::Square)
                    );
                }
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_mixed_quote_list_ancestry_positions_text_and_borders() {
        for (prefix, github_x, night_x, github_bars, night_bars) in [
            ("- > ", 49.0, 62.0, vec![30.0], vec![30.0]),
            ("> - ", 49.0, 92.0, vec![0.0], vec![30.0]),
            ("> - > ", 68.0, 124.0, vec![0.0, 49.0], vec![30.0, 92.0]),
            ("- > - > ", 98.0, 124.0, vec![30.0, 79.0], vec![30.0, 92.0]),
            ("> > - ", 68.0, 154.0, vec![0.0, 19.0], vec![30.0, 92.0]),
        ] {
            let source = format!("{prefix}first word\n\nend\n");
            for (theme, x, bars) in [
                (yu_core::ThemeId::Github, github_x, &github_bars),
                (yu_core::ThemeId::Night, night_x, &night_bars),
            ] {
                for zoom in [0.75, 1.0, 1.5] {
                    let (shaper, _, config) =
                        core_text_layout(16.0 * zoom, 600.0, theme).expect("font");
                    let mut document = yu_editor::EditorDocument::new(source.clone());
                    document
                        .set_selection(
                            yu_editor::EditorSelection::cursor(
                                &document.snapshot(),
                                yu_core::ByteOffset::new(source.len() as u64),
                                yu_editor::CaretAffinity::Downstream,
                            )
                            .expect("selection"),
                        )
                        .expect("EOF");
                    document
                        .set_viewport_config(yu_editor::ViewportConfig::new(
                            config,
                            16.0 * zoom,
                            0.0,
                        ))
                        .expect("config");
                    let snapshot = document
                        .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                        .expect("snapshot");
                    let first = snapshot
                        .block_for_source(yu_core::ByteOffset::new(prefix.len() as u64))
                        .expect("body");
                    let markers: Vec<_> = first.layout().ornaments().markers().collect();
                    assert_eq!(
                        markers.len(),
                        prefix.matches('-').count(),
                        "all list ancestors must render"
                    );
                    for marker in markers {
                        assert_eq!(
                            &source[marker.source().start().get() as usize
                                ..marker.source().end().get() as usize],
                            "-"
                        );
                    }
                    let offset = yu_core::ByteOffset::new(prefix.len() as u64);
                    let placed = snapshot.block_for_source(offset).expect("text");
                    let caret = placed
                        .layout()
                        .caret_for_source(offset, Bias::After)
                        .expect("caret");
                    assert!(
                        (caret.point().x() - x * zoom).abs() < 0.001,
                        "{prefix:?} {theme:?} {zoom}: {} expected {}",
                        caret.point().x(),
                        x * zoom
                    );
                    let actual: Vec<_> = snapshot
                        .containers()
                        .iter()
                        .filter_map(|c| c.quote_bar)
                        .collect();
                    assert_eq!(actual.len(), bars.len());
                    for (bar, expected) in actual.iter().zip(bars) {
                        assert!(
                            (bar.x() - expected * zoom).abs() < 0.001,
                            "{prefix:?} {theme:?}: bar={bar:?}"
                        );
                    }
                    let interior = placed
                        .layout()
                        .caret_for_source(yu_core::ByteOffset::new(offset.get() + 2), Bias::After)
                        .expect("interior");
                    assert_eq!(
                        placed
                            .layout()
                            .hit_test(interior.point())
                            .expect("hit")
                            .source(),
                        interior.source()
                    );
                    assert_eq!(document.snapshot().as_str(), source);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_ordered_ancestor_markers_keep_distinct_numbers_and_sources() {
        let source = "3. > 7. first\n1. > 9. second\n\nend\n";
        for theme in [yu_core::ThemeId::Github, yu_core::ThemeId::Night] {
            for zoom in [0.75, 1.0, 1.5] {
                let (shaper, _, config) =
                    core_text_layout(16.0 * zoom, 600.0, theme).expect("font");
                let mut document = yu_editor::EditorDocument::new(source);
                document
                    .set_selection(
                        yu_editor::EditorSelection::cursor(
                            &document.snapshot(),
                            yu_core::ByteOffset::new(source.len() as u64),
                            yu_editor::CaretAffinity::Downstream,
                        )
                        .expect("selection"),
                    )
                    .expect("EOF");
                document
                    .set_viewport_config(yu_editor::ViewportConfig::new(config, 16.0 * zoom, 0.0))
                    .expect("config");
                let snapshot = document
                    .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                    .expect("snapshot");
                for (word, labels, spellings) in [
                    ("first", ["3.", "7."], ["3.", "7."]),
                    ("second", ["4.", "9."], ["1.", "9."]),
                ] {
                    let offset = yu_core::ByteOffset::new(source.find(word).expect("word") as u64);
                    let layout = snapshot.block_for_source(offset).expect("block").layout();
                    let markers: Vec<_> = layout.ornaments().markers().collect();
                    assert_eq!(markers.len(), 2);
                    assert!(markers[0].x() + markers[0].advance() < markers[1].x());
                    for ((marker, label), spelling) in markers.iter().zip(labels).zip(spellings) {
                        assert_eq!(marker.text(), label);
                        assert_eq!(
                            &source[marker.source().start().get() as usize
                                ..marker.source().end().get() as usize],
                            spelling
                        );
                        assert!(
                            layout
                                .glyphs()
                                .iter()
                                .any(|glyph| glyph.source() == marker.source())
                        );
                    }
                    let caret = layout
                        .caret_for_source(yu_core::ByteOffset::new(offset.get() + 2), Bias::After)
                        .expect("caret");
                    assert_eq!(
                        layout.hit_test(caret.point()).expect("hit").source(),
                        caret.source()
                    );
                }
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_bidi_code_backgrounds_preserve_plain_text_and_source_hits() {
        for text in [
            "`abc א`בג def דהו xyz",
            "אב`ג def ד`הו xyz",
            "שלום `abc עו`לם def",
            "hello `مرحبا world أ`هلا",
            "`é אב`ג 👨‍👩‍👧‍👦 end",
        ] {
            let source = format!("{text}\n\nend\n");
            for theme in [yu_core::ThemeId::Github, yu_core::ThemeId::Night] {
                for zoom in [0.75, 1.0, 1.5] {
                    for width in [120.0, 600.0] {
                        let (shaper, _, config) =
                            core_text_layout(16.0 * zoom, width, theme).expect("font");
                        let mut document = yu_editor::EditorDocument::new(source.clone());
                        document
                            .set_selection(
                                yu_editor::EditorSelection::cursor(
                                    &document.snapshot(),
                                    yu_core::ByteOffset::new(source.len() as u64),
                                    yu_editor::CaretAffinity::Downstream,
                                )
                                .expect("caret"),
                            )
                            .expect("selection");
                        document
                            .set_viewport_config(yu_editor::ViewportConfig::new(
                                config,
                                16.0 * zoom,
                                0.0,
                            ))
                            .expect("config");
                        let snapshot = document
                            .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                            .expect("snapshot");
                        let layout = snapshot
                            .block_for_source(yu_core::ByteOffset::ZERO)
                            .expect("paragraph")
                            .layout();
                        let boxes = layout.inline_boxes().expect("inline boxes");
                        assert!(!boxes.is_empty());
                        if width == 600.0 && text.starts_with(['א', 'ש']) {
                            let last_latin = layout
                                .clusters()
                                .iter()
                                .filter(|cluster| !cluster.is_line_break())
                                .max_by(|a, b| (a.x() + a.width()).total_cmp(&(b.x() + b.width())))
                                .expect("rightmost cluster");
                            assert_eq!(
                                &source[last_latin.source().start().get() as usize
                                    ..last_latin.source().end().get() as usize],
                                if text.starts_with('א') { "z" } else { "f" },
                                "Reference prose has an LTR paragraph even when it starts with Hebrew"
                            );
                        }
                        for cluster in layout.clusters() {
                            if cluster.is_line_break() || cluster.width() <= 0.001 {
                                continue;
                            }
                            let line = &layout.lines()[cluster.line()];
                            let fragments: Vec<_> = boxes
                                .iter()
                                .filter(|f| {
                                    f.bounds.y() < line.y() + line.height()
                                        && f.bounds.bottom() > line.y()
                                })
                                .collect();
                            if cluster.style() != yu_core::TextStyle::Code {
                                for f in fragments {
                                    let overlap =
                                        f.bounds.right().min(cluster.x() + cluster.width())
                                            - f.bounds.x().max(cluster.x());
                                    assert!(
                                        overlap <= 0.01,
                                        "{text:?} {theme:?} zoom={zoom} width={width}: background crosses {:?}",
                                        cluster.source()
                                    );
                                }
                            }
                            let hit = layout
                                .hit_test(yu_core::Point::new(
                                    cluster.x() + cluster.width() * 0.25,
                                    line.y() + line.height() * 0.5,
                                ))
                                .expect("hit");
                            // Hidden delimiters and a bidi seam may map one
                            // physical edge to another logical source boundary.
                            // The canonical caret must remain near the clicked
                            // grapheme (including code padding) and round-trip.
                            let padding = (theme.inline_padding().0 + theme.inline_border()) * zoom;
                            let clicked_x = cluster.x() + cluster.width() * 0.25;
                            assert!(
                                (hit.point().x() - clicked_x).abs()
                                    <= cluster.width() * 0.5 + padding + 0.01,
                                "{text:?}: hit {:?} too far from cluster {cluster:?}",
                                hit.point()
                            );
                            assert_eq!(
                                layout
                                    .visual()
                                    .visual_to_source(hit.visual(), hit.bias())
                                    .expect("canonical source"),
                                hit.source()
                            );
                            let caret = layout
                                .caret_for_source(hit.source(), hit.bias())
                                .expect("source caret");
                            assert!(
                                (caret.point().x() - hit.point().x()).abs() < 0.01
                                    && (caret.point().y() - hit.point().y()).abs() < 0.01
                            );
                        }
                        assert_eq!(document.snapshot().as_str(), source);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_nested_task_keeps_ancestor_markers_and_checkbox_source_geometry() {
        for (prefix, labels) in [
            ("- > - [ ] ", vec!["•"]),
            ("> 3. > - [ ] ", vec!["3."]),
            ("3. > 7. > - [ ] ", vec!["3.", "7."]),
        ] {
            let source = format!("{prefix}todo 中文\n\nend\n");
            for theme in [yu_core::ThemeId::Github, yu_core::ThemeId::Night] {
                for zoom in [0.75, 1.0, 1.5] {
                    let (shaper, _, config) =
                        core_text_layout(16.0 * zoom, 600.0, theme).expect("font");
                    let mut document = yu_editor::EditorDocument::new(source.clone());
                    document
                        .set_viewport_config(yu_editor::ViewportConfig::new(
                            config,
                            16.0 * zoom,
                            0.0,
                        ))
                        .expect("config");
                    for focus in [source.len(), prefix.len() + 2] {
                        document
                            .set_selection(
                                yu_editor::EditorSelection::cursor(
                                    &document.snapshot(),
                                    yu_core::ByteOffset::new(focus as u64),
                                    yu_editor::CaretAffinity::Downstream,
                                )
                                .expect("caret"),
                            )
                            .expect("selection");
                        let snapshot = document
                            .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                            .expect("layout");
                        let layout = snapshot
                            .block_for_source(yu_core::ByteOffset::new(prefix.len() as u64))
                            .expect("task")
                            .layout();
                        assert_eq!(layout.checkboxes().len(), 1, "{prefix}");
                        let checkbox = layout.checkboxes()[0];
                        assert_eq!(
                            &source[checkbox.source().start().get() as usize
                                ..checkbox.source().end().get() as usize],
                            "[ ]"
                        );
                        let markers: Vec<_> = layout.ornaments().markers().collect();
                        assert_eq!(markers.len(), labels.len(), "{prefix} {theme:?}");
                        assert!(
                            (checkbox.bounds().width() - theme.task_checkbox_size() * zoom).abs()
                                < 0.001
                        );
                        if focus == source.len() {
                            let expected_x = match (prefix, theme) {
                                ("- > - [ ] ", yu_core::ThemeId::Github) => 79.0,
                                ("- > - [ ] ", yu_core::ThemeId::Night) => 92.0,
                                ("> 3. > - [ ] ", yu_core::ThemeId::Github) => 98.0,
                                ("> 3. > - [ ] ", yu_core::ThemeId::Night) => 154.0,
                                (_, yu_core::ThemeId::Github) => 128.0,
                                (_, yu_core::ThemeId::Night) => 154.0,
                                (_, yu_core::ThemeId::YuLight | yu_core::ThemeId::YuDark) => {
                                    unreachable!("this fixture records classic-theme geometry")
                                }
                            };
                            let start = layout
                                .caret_for_source(
                                    yu_core::ByteOffset::new(prefix.len() as u64),
                                    Bias::After,
                                )
                                .expect("task text");
                            assert!(
                                (start.point().x() - expected_x * zoom).abs() < 0.001,
                                "{prefix} {theme:?}: {:?}",
                                start.point()
                            );
                            assert!(
                                (checkbox.bounds().x() - (expected_x - 20.8) * zoom).abs() < 0.001
                            );
                        }
                        assert!(
                            layout.ornaments().marker().is_none(),
                            "checkbox owns the innermost gutter"
                        );
                        for (marker, label) in markers.iter().zip(&labels) {
                            if *label != "•" {
                                assert_eq!(marker.text(), *label);
                            }
                            assert!(marker.x() + marker.advance() < checkbox.bounds().x());
                        }
                        if focus == source.len() {
                            assert!(!layout.visual().text().contains('>'));
                        }
                        assert!(!layout.visual().text().contains('-'));
                        let caret = layout
                            .caret_for_source(
                                yu_core::ByteOffset::new((prefix.len() + 2) as u64),
                                Bias::After,
                            )
                            .expect("caret");
                        assert_eq!(
                            layout.hit_test(caret.point()).expect("hit").source(),
                            caret.source()
                        );
                    }
                    assert_eq!(document.snapshot().as_str(), source);
                    document.toggle_task(0).expect("toggle");
                    assert_eq!(
                        document.snapshot().as_str(),
                        source.replacen("[ ]", "[x]", 1)
                    );
                    document.undo().expect("undo");
                    assert_eq!(document.snapshot().as_str(), source);
                    document.redo().expect("redo");
                    assert_eq!(
                        document.snapshot().as_str(),
                        source.replacen("[ ]", "[x]", 1)
                    );
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_list_contained_quote_margins_override_theme_root_margins() {
        let source = "- intro\n\n  > quoted text\n\n  tail\n\nend\n";
        for theme in [yu_core::ThemeId::Github, yu_core::ThemeId::Night] {
            for zoom in [0.75, 1.0, 1.5] {
                let (shaper, _, config) =
                    core_text_layout(16.0 * zoom, 600.0, theme).expect("font");
                let mut document = yu_editor::EditorDocument::new(source);
                document
                    .set_viewport_config(yu_editor::ViewportConfig::new(config, 16.0 * zoom, 0.0))
                    .expect("config");
                let snapshot = document
                    .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                    .expect("snapshot");
                let block = |text| {
                    snapshot
                        .block_for_source(yu_core::ByteOffset::new(
                            source.find(text).expect("source") as u64,
                        ))
                        .expect("block")
                };
                let bar = snapshot
                    .containers()
                    .iter()
                    .find_map(|c| c.quote_bar)
                    .expect("bar");
                let intro = block("intro");
                assert!(
                    (bar.y() - intro.content_y() - intro.layout().height() - 16.0 * zoom).abs()
                        < 0.001,
                    "{theme:?} before"
                );
                assert!(
                    (block("tail").content_y() - bar.bottom() - 16.0 * zoom).abs() < 0.001,
                    "{theme:?} after"
                );
                assert!((bar.x() - 30.0 * zoom).abs() < 0.001);
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_quote_boxes_share_theme_margins_borders_and_carets() {
        for (theme, margin, padding, border, before, after) in [
            (yu_core::ThemeId::Github, 0.0, 15.0, 4.0, 12.8, 12.8),
            (yu_core::ThemeId::Night, 30.0, 30.0, 2.0, 35.0, 30.0),
        ] {
            for zoom in [0.75, 1.0, 1.5] {
                for source in [
                    "> quoted text\n\nafter\n",
                    "before\n\n> quoted text\n\nafter\n",
                ] {
                    let (shaper, _, config) =
                        core_text_layout(16.0 * zoom, 600.0, theme).expect("font");
                    let mut document = yu_editor::EditorDocument::new(source);
                    // Match the reference's inactive quote; a caret on `>` reveals source syntax.
                    document
                        .set_selection(
                            yu_editor::EditorSelection::cursor(
                                &document.snapshot(),
                                yu_core::ByteOffset::new(source.len() as u64),
                                yu_editor::CaretAffinity::Downstream,
                            )
                            .expect("selection"),
                        )
                        .expect("caret at EOF");
                    document
                        .set_viewport_config(yu_editor::ViewportConfig::new(
                            config,
                            16.0 * zoom,
                            0.0,
                        ))
                        .expect("config");
                    let snapshot = document
                        .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                        .expect("snapshot");
                    let quote = snapshot
                        .containers()
                        .iter()
                        .find(|c| c.quote_bar.is_some())
                        .expect("quote");
                    let bar = quote.quote_bar.expect("bar");
                    assert!((bar.x() - margin * zoom).abs() < 0.001);
                    assert!((bar.width() - border * zoom).abs() < 0.001);
                    let offset =
                        yu_core::ByteOffset::new((source.find("quoted").expect("text") + 2) as u64);
                    let placed = snapshot.block_for_source(offset).expect("quote block");
                    assert!((bar.y() - placed.content_y()).abs() < 0.001);
                    let left = placed
                        .layout()
                        .caret_for_source(
                            yu_core::ByteOffset::new(source.find("quoted").expect("text") as u64),
                            Bias::After,
                        )
                        .expect("text start")
                        .point()
                        .x();
                    assert!(
                        (left - (margin + border + padding) * zoom).abs() < 0.001,
                        "{theme:?} {zoom} {source:?}: left={left}"
                    );
                    let caret = placed
                        .layout()
                        .caret_for_source(offset, Bias::After)
                        .expect("caret");
                    assert_eq!(
                        placed
                            .layout()
                            .hit_test(caret.point())
                            .expect("hit")
                            .source(),
                        offset
                    );
                    let preceding_height = if source.starts_with("before") {
                        snapshot.blocks()[0].layout().height()
                    } else {
                        0.0
                    };
                    assert!((bar.y() - preceding_height - before * zoom).abs() < 0.001);
                    let next = snapshot
                        .block_for_source(yu_core::ByteOffset::new(
                            source.find("after").expect("after") as u64,
                        ))
                        .expect("next");
                    assert!((next.content_y() - bar.bottom() - after * zoom).abs() < 0.001);
                    assert_eq!(document.snapshot().as_str(), source);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn source_mode_coretext_keeps_literal_line_geometry_and_hits() {
        let source = "# Heading\n\n- **item**\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n| - | - |\n| x | y |";
        let (shaper, _, config) =
            core_text_layout(16.0, 760.0, yu_core::ThemeId::YuLight).expect("font");
        let mut doc = yu_editor::EditorDocument::new(source);
        doc.set_viewport_config(yu_editor::ViewportConfig::new(config, 16.0, 0.0))
            .expect("config");
        doc.set_source_mode(true).expect("source mode");
        let snapshot = doc
            .prepare_layout_snapshot(ViewportSpan::new(0.0, 4000.0), &shaper)
            .expect("layout");
        let mut offset = 0;
        let mut previous_y: Option<f32> = None;
        let first = snapshot
            .block_for_source(yu_core::ByteOffset::ZERO)
            .expect("first line");
        let xs: Vec<_> = (0..=9)
            .map(|i| {
                first
                    .layout()
                    .caret_for_source(yu_core::ByteOffset::new(i), Bias::After)
                    .expect("ASCII caret")
                    .point()
                    .x()
            })
            .collect();
        let advance = xs[1] - xs[0];
        for pair in xs.windows(2) {
            assert!(
                (pair[1] - pair[0] - advance).abs() < 0.05,
                "source font must be monospaced"
            );
        }
        for line in source.split('\n') {
            let at = yu_core::ByteOffset::new(offset as u64);
            let block = snapshot.block_for_source(at).expect("line block");
            let caret = block
                .layout()
                .caret_for_source(at, Bias::After)
                .expect("line caret");
            let y = block.content_y() + caret.point().y();
            if let Some(previous) = previous_y {
                assert!(
                    (y - previous - 26.0_f32).abs() < 0.05,
                    "line {line:?}: y={y} previous={previous}"
                );
            }
            assert!(
                caret.point().x().abs() < 0.05,
                "source line has Markdown indentation: {line}"
            );
            if !line.is_empty() {
                assert_eq!(
                    block
                        .layout()
                        .hit_test(caret.point())
                        .expect("hit")
                        .source(),
                    at
                );
            }
            previous_y = Some(y);
            offset += line.len() + 1;
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_theme_line_struts_preserve_fractional_advance() {
        let source = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu
";
        for (theme, heights) in [
            (yu_core::ThemeId::Github, [19.2, 25.6, 38.4]),
            (yu_core::ThemeId::Night, [19.5, 26.0, 39.0]),
        ] {
            for (zoom, expected) in [0.75, 1.0, 1.5].into_iter().zip(heights) {
                let (shaper, _, config) =
                    core_text_layout(16.0 * zoom, 150.0 * zoom, theme).expect("font");
                let mut document = yu_editor::EditorDocument::new(source);
                document
                    .set_viewport_config(yu_editor::ViewportConfig::new(config, 16.0 * zoom, 0.0))
                    .expect("config");
                let snapshot = document
                    .prepare_layout_snapshot(ViewportSpan::new(0.0, 2000.0), &shaper)
                    .expect("snapshot");
                let layout = snapshot.blocks()[0].layout();
                assert!(
                    layout.lines().len() >= 8,
                    "exercise cumulative line advance"
                );
                for (index, line) in layout.lines().iter().enumerate() {
                    assert!((line.bounds().height() - expected).abs() < 0.001);
                    assert!((line.bounds().y() - index as f32 * expected).abs() < 0.001);
                }
                for cluster in layout
                    .clusters()
                    .iter()
                    .filter(|cluster| !cluster.visual().is_empty())
                {
                    let caret = layout
                        .caret_for_source(cluster.source().start(), Bias::After)
                        .expect("caret");
                    assert_eq!(
                        layout.hit_test(caret.point()).expect("hit").source(),
                        caret.source()
                    );
                }
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_queries_share_painted_code_and_heading_geometry() {
        let path = std::env::temp_dir().join(format!("yu-shared-geometry-{}.md", temp_id()));
        let source = "# Heading\n\nparagraph above\n\n```swift\nlet value = 1\n```\n";
        fs::write(&path, source).expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let offset = source.find("value").expect("code") + 2;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    offset as u64,
                    offset as u64,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let session = unsafe { &mut *raw };
        let (shaper, metrics, _) =
            core_text_layout(16.0, 500.0, yu_core::ThemeId::Github).expect("CoreText");
        macos_publish_viewport_config(session, 500.0, metrics, yu_core::ThemeId::Github)
            .expect("config");
        let geometry = macos_query_layout_snapshot(session, ViewportSpan::new(0.0, 800.0), &shaper)
            .expect("frame geometry");
        let placed = geometry
            .block_for_source(yu_core::ByteOffset::new(offset as u64))
            .expect("code");
        let native = placed
            .layout()
            .caret_for_source(yu_core::ByteOffset::new(offset as u64), Bias::After)
            .expect("code caret");
        let point = placed.document_point(native.point());
        // A notification from another document is not a geometry change here.
        yu_render_macos::notify_resource_completion();
        let mut caret = YuStorageBlockCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_source_caret(raw, 0, offset as u64, 1, 16.0, 500.0, &mut caret)
            },
            YU_STORAGE_OK
        );
        assert!(
            (caret.caret_y - point.y()).abs() < 0.01,
            "must include preceding blocks and code padding"
        );
        assert!((caret.caret_x - point.x()).abs() < 0.01);
        assert!((caret.caret_height - placed.caret_height(native)).abs() < 0.01);
        let mut hit = YuStorageProjectionHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_projection_hit_test(
                    raw,
                    0,
                    point.x(),
                    point.y() + caret.caret_height * 0.25,
                    16.0,
                    500.0,
                    &mut hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(hit.source_utf16, offset as u64);
        let session = unsafe { &mut *raw };
        let current = session
            .session
            .document()
            .editor()
            .current_layout_snapshot()
            .expect("retained geometry");
        assert!(std::sync::Arc::ptr_eq(&geometry, &current));
        // The heading caret uses its own native line, not the body strut.
        let heading = geometry.block(0).expect("heading");
        let heading_caret = heading
            .layout()
            .caret_for_source(yu_core::ByteOffset::new(4), Bias::After)
            .expect("heading caret");
        assert_eq!(
            unsafe { yu_storage_session_source_caret(raw, 0, 4, 1, 16.0, 500.0, &mut caret) },
            YU_STORAGE_OK
        );
        assert!((caret.caret_height - heading.caret_height(heading_caret)).abs() < 0.01);
        assert!(caret.caret_height > metrics.line_height());
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// `macos_source_caret` 让平台在不知道 block 归属的情况下取得 caret 几何。
    /// AppKit 的 `firstRect(forCharacterRange:)` 需要它来定位 IME 候选窗：
    /// TextKit 排的是 canonical source，屏幕显示的是投影结果，两者字符位置
    /// 不对应，用默认实现会让候选窗偏离真实插入点（不变量 H3、I1）。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_source_caret_resolves_owning_block_and_is_revision_bound() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-source-caret-{id}.md"));
        let source = "# 标题\n\nParagraph **粗体** and 日本語🙂\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let source_start = source.find("**粗体**").expect("strong marker");
        let source_utf16 = source[..source_start].encode_utf16().count() as u64;

        let mut caret = YuStorageBlockCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_source_caret(
                    raw,
                    0,
                    source_utf16,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                    14.0,
                    500.0,
                    &mut caret,
                )
            },
            YU_STORAGE_OK
        );
        // 块归属必须由 Rust 按 offset 解析出来：目标偏移在第 3 个块里。
        assert_eq!(caret.block_index, 2);
        assert_eq!(caret.source_utf16, source_utf16);
        assert_eq!(caret.shaped, 1);
        assert!(caret.caret_height > 0.0);
        assert!(caret.caret_x.is_finite() && caret.caret_y.is_finite());

        // 文档开头属于另一个 block，块归属必须真的按 offset 解析而非写死。
        let mut first = YuStorageBlockCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_source_caret(
                    raw,
                    0,
                    0,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                    14.0,
                    500.0,
                    &mut first,
                )
            },
            YU_STORAGE_OK
        );
        assert_ne!(first.block_index, caret.block_index);

        // 越界 offset 必须被拒绝，且不写入半成品结果。
        let out_of_range = source.encode_utf16().count() as u64 + 1;
        let mut rejected = YuStorageBlockCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_source_caret(
                    raw,
                    0,
                    out_of_range,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                    14.0,
                    500.0,
                    &mut rejected,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(rejected, YuStorageBlockCaret::default());

        // 编辑之后旧 Revision 的查询必须整体拒绝。
        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_source_caret(
                    raw,
                    0,
                    source_utf16,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                    14.0,
                    500.0,
                    &mut caret,
                )
            },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_composition_projection_is_generation_bound_and_preserves_source() {
        let id = temp_id();
        let path =
            std::env::temp_dir().join(format!("yu-storage-ffi-composition-projection-{id}.md"));
        let source = "before **x** after";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_begin_composition(
                    raw,
                    0,
                    9,
                    10,
                    "日本🙂".as_ptr(),
                    "日本🙂".len(),
                    2,
                    4,
                )
            },
            YU_STORAGE_OK
        );

        let mut projection = YuStorageCompositionProjection::default();
        assert_eq!(
            unsafe { yu_storage_session_composition_projection(raw, 0, &mut projection) },
            YU_STORAGE_OK
        );
        assert_eq!(projection.revision, 0);
        assert_eq!(projection.generation, 1);
        assert_eq!(projection.replacement_start_utf16, 9);
        assert_eq!(projection.replacement_end_utf16, 10);
        assert_eq!(projection.preedit_selection_start_utf16, 2);
        assert_eq!(projection.preedit_selection_end_utf16, 4);
        assert_eq!(projection.visual_selection_start_utf16, 9);
        assert_eq!(projection.visual_selection_end_utf16, 11);
        assert_eq!(projection.visual_replacement_start_utf16, 7);
        assert_eq!(projection.visual_replacement_end_utf16, 11);
        assert_eq!(projection.projected_utf16_length, 17);
        assert_eq!(
            projection.projected_utf8_length,
            "before 日本🙂 after".len() as u64
        );

        // preedit 投影文本与 composition caret 的几何都不再跨 ABI
        // （不变量 I3）；这里保留的是 generation 绑定与 source 不变。
        assert_eq!(
            unsafe {
                yu_storage_session_update_composition(
                    raw,
                    0,
                    projection.generation,
                    "日本語".as_ptr(),
                    "日本語".len(),
                    3,
                    3,
                )
            },
            YU_STORAGE_OK
        );
        let mut updated = projection;
        assert_eq!(
            unsafe { yu_storage_session_composition_projection(raw, 0, &mut updated) },
            YU_STORAGE_OK
        );
        assert_eq!(updated.generation, 2);
        assert_eq!(updated.projected_utf16_length, 16);
        // 旧 generation 的写入必须整体拒绝。
        assert_eq!(
            unsafe {
                yu_storage_session_update_composition(
                    raw,
                    0,
                    projection.generation,
                    "x".as_ptr(),
                    1,
                    1,
                    1,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );

        let mut canonical_required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source(raw, ptr::null_mut(), 0, &mut canonical_required)
            },
            YU_STORAGE_OK
        );
        let mut canonical = vec![0_u8; canonical_required];
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source(
                    raw,
                    canonical.as_mut_ptr(),
                    canonical.len(),
                    &mut canonical_required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            std::str::from_utf8(&canonical).expect("canonical UTF-8"),
            source
        );

        let state = unsafe { yu_storage_session_cancel_composition(raw, 0, updated.generation) };
        assert_eq!(state, YU_STORAGE_OK);
        assert_eq!(
            unsafe { yu_storage_session_composition_projection(raw, 0, &mut updated) },
            YU_STORAGE_NO_OVERLAY
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_projection_visual_mirror_maps_caret_and_selection_back_to_source() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-visual-mirror-{id}.md"));
        let source = "before **粗体** after\n日本🙂";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        // 投影文本不再跨 ABI（不变量 I3）；直接查内部投影。
        let projected = {
            // SAFETY: `raw` is a live session handle owned by this test.
            let session = unsafe { raw.as_mut() }.expect("session");
            let projection = session
                .session
                .visual_text_for_visual_state()
                .expect("projection");
            projected_utf8(&projection)
        };
        let source_strong = source.find("**粗体**").expect("source strong") as u64;
        let source_strong_end = source_strong + "**粗体**".len() as u64;
        let visual_strong = projected.find("粗体").expect("visual strong") as u64;
        let visual_strong_end = visual_strong + "粗体".len() as u64;
        let source_start_utf16 = source[..source_strong as usize].encode_utf16().count() as u64;
        let source_end_utf16 = source[..source_strong_end as usize].encode_utf16().count() as u64;
        let visual_start_utf16 = projected[..visual_strong as usize].encode_utf16().count() as u64;
        let visual_end_utf16 = projected[..visual_strong_end as usize]
            .encode_utf16()
            .count() as u64;

        // 折叠选区就是「一个光标」的映射，与非折叠区间走同一条路径。
        let mut collapsed = YuStorageProjectionSourceSelection {
            revision: 99,
            ..YuStorageProjectionSourceSelection::default()
        };
        assert_eq!(
            unsafe {
                yu_storage_session_projection_source_selection(
                    raw,
                    0,
                    visual_start_utf16,
                    visual_start_utf16,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                    &mut collapsed,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(collapsed.revision, 0);
        assert_eq!(collapsed.visual_start_utf16, visual_start_utf16);
        assert_eq!(collapsed.source_start_utf16, source_start_utf16);
        assert_eq!(collapsed.round_trip_visual_start_utf16, visual_start_utf16);

        let mut selection = YuStorageProjectionSourceSelection::default();
        assert_eq!(
            unsafe {
                yu_storage_session_projection_source_selection(
                    raw,
                    0,
                    visual_start_utf16,
                    visual_end_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut selection,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(selection.revision, 0);
        assert_eq!(selection.visual_start_utf16, visual_start_utf16);
        assert_eq!(selection.visual_end_utf16, visual_end_utf16);
        assert_eq!(selection.source_start_utf16, source_start_utf16);
        assert_eq!(selection.source_end_utf16, source_end_utf16);
        assert_eq!(selection.round_trip_visual_start_utf16, visual_start_utf16);
        assert_eq!(selection.round_trip_visual_end_utf16, visual_end_utf16);

        // surrogate 中间位置不得穿过 ABI（不变量 I4）。
        let emoji_visual_start = projected.find("🙂").expect("emoji") as u64;
        let emoji_utf16 = projected[..emoji_visual_start as usize]
            .encode_utf16()
            .count() as u64;
        let mut surrogate = YuStorageProjectionSourceSelection::default();
        assert_eq!(
            unsafe {
                yu_storage_session_projection_source_selection(
                    raw,
                    0,
                    emoji_utf16 + 1,
                    emoji_utf16 + 1,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut surrogate,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(surrogate, YuStorageProjectionSourceSelection::default());

        selection.revision = 99;
        assert_eq!(
            unsafe {
                yu_storage_session_projection_source_selection(
                    raw,
                    0,
                    visual_end_utf16,
                    visual_start_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut selection,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(selection, YuStorageProjectionSourceSelection::default());

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_projection_source_selection(
                    raw,
                    0,
                    visual_start_utf16,
                    visual_end_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut selection,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(selection, YuStorageProjectionSourceSelection::default());

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// 一帧必须自己报告可见范围内是否还有未落定的资源。
    ///
    /// 这个标志决定平台要不要再提交一次去收割 worker 结果。恒为 0 时图片会
    /// 永远停在 placeholder 上——没有报错，只是图不出来；恒为 1 时平台会一直
    /// 空转轮询。两种都不会有任何日志。
    ///
    /// 反向验证：让 `macos_frame_needs_resource_refresh` 恒返回 false，
    /// 第二段断言失败；恒返回 true，第一段断言失败。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_frame_reports_pending_resources_for_the_visible_range() {
        fn frame_pending(source: &str, name: &str) -> u8 {
            let id = temp_id();
            let path = std::env::temp_dir().join(format!("yu-storage-ffi-{name}-{id}.md"));
            fs::write(&path, source).expect("fixture");
            let path_bytes = path.to_string_lossy().as_bytes().to_vec();
            let mut raw = ptr::null_mut();
            assert_eq!(
                unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
                YU_STORAGE_OK
            );
            let mut frame = YuStorageMacosRenderHostSnapshot::default();
            assert_eq!(
                unsafe {
                    yu_storage_session_macos_render_host_frame(
                        raw,
                        0,
                        14.0,
                        500.0,
                        0.0,
                        240.0,
                        0,
                        YU_STORAGE_APPEARANCE_LIGHT,
                        &mut frame,
                    )
                },
                YU_STORAGE_OK
            );
            assert_eq!(frame.published, 1);
            let session = unsafe { &*raw };
            let host = session.macos_render_host.as_ref().expect("render host");
            assert_eq!(
                host.resource_refresh_pending,
                frame.resource_refresh_pending != 0
            );
            let retained =
                macos_render_host_snapshot(host, session.session.composition_generation())
                    .expect("retained snapshot");
            assert_eq!(
                retained.resource_refresh_pending,
                frame.resource_refresh_pending
            );
            unsafe { yu_storage_session_destroy(raw) };
            fs::remove_file(path).expect("cleanup");
            frame.resource_refresh_pending
        }

        // 纯文本没有任何可调度的资源，第一帧就是终态。恒为 1 会让平台一直轮询。
        assert_eq!(
            frame_pending("# \u{6807}\u{9898}\n\nparagraph\n", "frame-no-resource"),
            0,
            "没有资源的文档不得要求再次提交"
        );

        // 图片在第一帧一定还没解码完（未知或在途），必须要求再提交一次；
        // 否则它会永远停在 placeholder 上，而且不报错。
        assert_eq!(
            frame_pending("![logo](assets/yu.png)\n", "frame-image"),
            1,
            "可见图片未就绪时必须要求再次提交"
        );

        // 普通代码块不是内嵌资源，不得因为它是 fenced code 就要求重试。
        assert_eq!(
            frame_pending("```rust\nfn main() {}\n```\n", "frame-code"),
            0,
            "普通代码块不是可调度资源"
        );

        // 空 body 的 math 块在缓存里落到 FAILED，需要重试；这一条同时守住
        // info string 的语言识别——识别不出来就会被当成普通代码块而报 0。
        assert_eq!(
            frame_pending("```math\n```\n", "frame-math"),
            1,
            "未就绪的内嵌资源必须要求再次提交"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_failed_image_settles_then_explicit_retry_loads_restored_file_without_editing() {
        let directory = std::env::temp_dir().join(format!("yu-image-retry-{}", temp_id()));
        fs::create_dir(&directory).expect("image retry fixture");
        let path = directory.join("document.md");
        let source = "![missing](restored.png)\r\n\r\n原文不变\r\n";
        let original = [b"\xef\xbb\xbf".as_slice(), source.as_bytes()].concat();
        fs::write(&path, &original).expect("image retry fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let mut info = YuStorageImageProperties::default();
        assert_eq!(
            unsafe { yu_storage_session_image_properties(raw, 0, 0, &mut info) },
            YU_STORAGE_OK
        );
        let mut status = 255;
        assert_eq!(
            unsafe { yu_storage_session_image_resource_status(raw, &info, &mut status) },
            YU_STORAGE_OK
        );
        assert_eq!(status, YU_STORAGE_IMAGE_RESOURCE_UNKNOWN);
        let frame = |raw| {
            let mut frame = YuStorageMacosRenderHostSnapshot::default();
            assert_eq!(
                unsafe {
                    yu_storage_session_macos_render_host_frame(
                        raw,
                        0,
                        16.0,
                        500.0,
                        0.0,
                        240.0,
                        0,
                        YU_STORAGE_APPEARANCE_LIGHT,
                        &mut frame,
                    )
                },
                YU_STORAGE_OK
            );
            frame
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let next = frame(raw);
            assert_eq!(
                unsafe { yu_storage_session_image_resource_status(raw, &info, &mut status) },
                YU_STORAGE_OK
            );
            if status == YU_STORAGE_IMAGE_RESOURCE_FAILED && next.resource_refresh_pending == 0 {
                assert_eq!(next.resource_retry_pending, 0);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "failed image must stop requesting frames after retry budget"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut stale = info;
        stale.revision = 1;
        assert_eq!(
            unsafe { yu_storage_session_retry_image(raw, &stale) },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe { yu_storage_session_image_resource_status(raw, &info, ptr::null_mut()) },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(
            frame(raw).resource_refresh_pending,
            0,
            "stale retry did not clear failure"
        );
        let png: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15,
            0, 1, 5, 1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        fs::write(directory.join("restored.png"), png).expect("image retry fixture");
        assert_eq!(
            unsafe { yu_storage_session_retry_image(raw, &info) },
            YU_STORAGE_OK
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let next = frame(raw);
            assert_eq!(
                unsafe { yu_storage_session_image_resource_status(raw, &info, &mut status) },
                YU_STORAGE_OK
            );
            if status == YU_STORAGE_IMAGE_RESOURCE_READY {
                assert_eq!(next.resource_refresh_pending, 0);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "restored file should load through the actual ImageIO worker"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let session = unsafe { &*raw };
        assert_eq!(session.session.snapshot().as_str(), source);
        assert_eq!(session.session.revision(), yu_core::Revision::INITIAL);
        assert!(!session.session.command_available(&EditorCommand::Undo));
        assert_eq!(
            session.session.selections().primary().focus(),
            yu_core::ByteOffset::ZERO
        );
        assert_eq!(fs::read(&path).expect("image retry fixture"), original);
        assert_eq!(
            unsafe { yu_storage_session_retry_image(raw, &info) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_image_resource_status(raw, &info, &mut status) },
            YU_STORAGE_OK
        );
        assert_eq!(
            status, YU_STORAGE_IMAGE_RESOURCE_READY,
            "repeat retry must retain successful decoded pixels"
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_dir_all(directory).expect("image retry fixture");
    }

    /// viewport 配置由 Rust 自己发布，且已经对齐时不得重建。
    ///
    /// 这条路径此前靠十份重复校验守着：平台没送对值就整个调用失败。现在校验
    /// 变成了发布，失败模式也随之变了——发布错了不会报错，只会让整份文档按
    /// 错误的行高与换行宽度排版。因此这里直接断言发布后的配置内容。
    ///
    /// 产品链路上，一份 CoreText 排不出来的文档必须仍然能出几何。
    ///
    /// 这一条守的是不变量 I5 在 shaping 上的落法。**它的判据必须在 FFI 这一层**
    /// ——`ShapingProvider` 的 `Err` 以前一路传成 `LayoutError::Shaping` →
    /// `EditorDocumentError::Layout` → 这里的 `storage_status`，于是平台侧收到
    /// 一个错误码，那一屏什么都不画。`yu-layout` 自己的用例证明得了「排不出来
    /// 的簇变成替代字形」，证明不了「这条错误码不再出现」。
    ///
    /// 语料是希伯来文，S7 第七刀 spike 实测 CoreText 连单独排一个字符都拒。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_caret_geometry_survives_a_script_core_text_refuses() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-unshapable-{id}.md"));
        fs::write(&path, "\u{05e9}\u{05dc}\u{05d5}\u{05dd}\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        // SAFETY: `raw` is a live session handle owned by this test.
        let session = unsafe { raw.as_mut() }.expect("session");

        let caret = macos_shaped_caret(session, 0, None, 0, 0, 14.0, 500.0)
            .expect("排不出来的脚本不该让 FFI 返回错误码");
        assert!(caret.caret_height > 0.0, "caret 必须有高度，否则等于没画");

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// 「已对齐就跳过」不是优化：`set_viewport_config` 会重建 `ViewportLayout`
    /// 并丢掉缓存的 block 高度，那是 J2 定位可见范围的依据。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_viewport_config_is_published_by_rust_and_kept_when_aligned() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-viewport-config-{id}.md"));
        fs::write(&path, "# \u{6807}\u{9898}\n\nparagraph\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        // SAFETY: `raw` is a live session handle owned by this test.
        let session = unsafe { raw.as_mut() }.expect("session");
        let (_, metrics, _) =
            core_text_layout(14.0, 500.0, yu_core::ThemeId::Github).expect("CoreText");

        macos_publish_viewport_config(session, 500.0, metrics, yu_core::ThemeId::Github)
            .expect("publish");
        let published = session.session.viewport_config().layout();
        assert!((published.max_width() - 500.0).abs() <= f32::EPSILON);
        assert!((published.line_height() - metrics.line_height()).abs() <= f32::EPSILON);
        assert!((published.default_advance() - metrics.default_advance()).abs() <= f32::EPSILON);

        // 用一个可辨认的 estimated_block_height 观察「跳过」是否真的发生。
        let marked = ViewportConfig::new(published, 987.0, 0.0);
        session.session.set_viewport_config(marked).expect("marked");
        macos_publish_viewport_config(session, 500.0, metrics, yu_core::ThemeId::Github)
            .expect("republish");
        assert!(
            (session.session.viewport_config().estimated_block_height() - 987.0).abs()
                <= f32::EPSILON,
            "配置已经对齐时不得重建 ViewportLayout"
        );

        // 换行宽度变化必须真正重新发布。
        macos_publish_viewport_config(session, 640.0, metrics, yu_core::ThemeId::Github)
            .expect("publish width");
        assert!(
            (session.session.viewport_config().layout().max_width() - 640.0).abs() <= f32::EPSILON,
            "宽度变化必须重新发布配置"
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// backing scale 非法时不得污染配置。
    ///
    /// 这个值会同时决定字形的取样倍率和后端的除数，取到 0 或 NaN 会让整帧
    /// 几何失效，因此宁可退回 1.0 按逻辑尺寸渲染（只是不够清晰）。
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_render_host_config_rejects_invalid_raster_scale() {
        let viewport = ViewportSpan::new(0.0, 240.0);
        for invalid in [0.0_f32, -1.0, f32::NAN, f32::INFINITY] {
            let config =
                macos_render_host_config(viewport, 14.0, 500.0, 240.0, invalid, Appearance::Light)
                    .expect("config should still build");
            assert_eq!(
                config.raster_scale(),
                1.0,
                "非法 raster scale {invalid} 应退回 1.0"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_table_resize_preview_reaches_retained_render_host_frame() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-table-resize-{id}.md"));
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        // 这些断言用行高做几何基准；配置的发布已经由 Rust 自己完成，
        // 这里只是取同一份度量来算期望值。
        let (_, metrics, _) =
            core_text_layout(14.0, 500.0, yu_core::ThemeId::Github).expect("CoreText");

        let mut first = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut first,
                )
            },
            YU_STORAGE_OK
        );
        let state = unsafe { raw.as_ref() }.expect("session");
        let divider = state
            .macos_render_host
            .as_ref()
            .and_then(|host| host.builder.last_publication())
            .and_then(|publication| {
                publication
                    .frame()
                    .scene()
                    .scene()
                    .primitives()
                    .iter()
                    .filter_map(|primitive| match primitive {
                        Primitive::Ornament(table)
                            if table.role() == yu_scene::OrnamentRole::Border
                                && table.bounds().x() > 0.0
                                && table.bounds().x() < 499.0 =>
                        {
                            Some(table.bounds().x() + table.bounds().width() * 0.5)
                        }
                        _ => None,
                    })
                    .next()
            })
            .expect("CoreText table divider");
        let mut accessibility_required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_accessibility_dividers(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    ptr::null_mut(),
                    0,
                    &mut accessibility_required,
                )
            },
            YU_STORAGE_OK
        );
        assert!(accessibility_required >= 1);
        let mut accessibility_dividers =
            vec![YuStorageTableResizeAccessibilityDivider::default(); accessibility_required];
        let mut accessibility_written = accessibility_required;
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_accessibility_dividers(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    accessibility_dividers.as_mut_ptr(),
                    accessibility_dividers.len(),
                    &mut accessibility_written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(accessibility_written, accessibility_required);
        let accessibility_divider = accessibility_dividers
            .iter()
            .find(|divider| divider.index == 0)
            .expect("first accessible table divider");
        assert_eq!(accessibility_divider.revision, 0);
        assert_eq!(accessibility_divider.block_index, 0);
        assert_eq!(accessibility_divider.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert!(accessibility_divider.column_count >= 2);
        assert!((accessibility_divider.x - divider).abs() < 0.01);
        assert!(accessibility_divider.height > 0.0);
        assert_eq!(state.session.snapshot().as_str(), source);
        let point_y = accessibility_divider.y + metrics.line_height() * 0.5;
        let mut hover = 9;
        for (revision, x, expected_status, expected_hit) in [
            (0, divider, YU_STORAGE_OK, 1),
            (0, 600.0, YU_STORAGE_OK, 0),
            (1, divider, YU_STORAGE_STALE_REVISION, 0),
            (0, f32::NAN, YU_STORAGE_EDITOR_ERROR, 0),
        ] {
            assert_eq!(
                unsafe {
                    yu_storage_session_table_resize_hover(
                        raw, revision, 14.0, 500.0, 0.0, 240.0, x, point_y, 0.2, &mut hover,
                    )
                },
                expected_status
            );
            assert_eq!(hover, expected_hit);
        }
        let mut document_hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_PROBE,
                    14.0,
                    500.0,
                    divider + 0.01,
                    point_y,
                    0.2,
                    0.0,
                    &mut document_hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(document_hit.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert_eq!(document_hit.index, 0);
        assert!((document_hit.position - divider).abs() < 0.01);
        let mut document_begin = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_BEGIN,
                    14.0,
                    500.0,
                    divider + 0.01,
                    point_y,
                    0.2,
                    divider + 0.01,
                    &mut document_begin,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(document_begin, document_hit);
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_CANCEL,
                    0.0,
                    &mut YuStorageTableResizeCommit::default(),
                )
            },
            YU_STORAGE_OK
        );
        let mut hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_BEGIN,
                    14.0,
                    500.0,
                    divider + 0.01,
                    point_y,
                    0.2,
                    divider + 0.01,
                    &mut hit,
                )
            },
            YU_STORAGE_OK
        );
        let mut preview = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_UPDATE,
                    divider + 1.01,
                    &mut preview,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(preview.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert!((preview.final_position - (divider + 1.0)).abs() < 0.01);
        let mut transient = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut transient,
                )
            },
            YU_STORAGE_OK
        );
        let state = unsafe { raw.as_ref() }.expect("session");
        let publication = state
            .macos_render_host
            .as_ref()
            .and_then(|host| host.builder.last_publication())
            .expect("retained publication");
        assert!(
            publication
                .frame()
                .scene()
                .scene()
                .primitives()
                .iter()
                .any(|primitive| {
                    matches!(
                        primitive,
                        Primitive::Ornament(table)
                            if table.role() == yu_scene::OrnamentRole::Border
                                && (table.bounds().x() + table.bounds().width() * 0.5 - (divider + 1.0)).abs() < 0.01
                    )
                })
        );
        assert_eq!(state.session.snapshot().as_str(), source);

        let mut committed = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_FINISH,
                    0.0,
                    &mut committed,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(committed, preview);
        let mut effective_required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_accessibility_dividers(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    ptr::null_mut(),
                    0,
                    &mut effective_required,
                )
            },
            YU_STORAGE_OK
        );
        let mut effective_dividers =
            vec![YuStorageTableResizeAccessibilityDivider::default(); effective_required];
        let mut effective_written = effective_required;
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_accessibility_dividers(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    effective_dividers.as_mut_ptr(),
                    effective_dividers.len(),
                    &mut effective_written,
                )
            },
            YU_STORAGE_OK
        );
        let effective_divider = effective_dividers
            .iter()
            .find(|divider| divider.index == 0)
            .expect("effective accessible table divider");
        assert_eq!(effective_written, effective_required);
        assert!((effective_divider.x - committed.final_position).abs() < 0.01);
        let mut effective_hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_PROBE,
                    14.0,
                    500.0,
                    effective_divider.x + 0.01,
                    point_y,
                    0.2,
                    0.0,
                    &mut effective_hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(effective_hit.index, 0);
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_action(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_CANCEL,
                    0.0,
                    &mut YuStorageTableResizeCommit::default(),
                )
            },
            YU_STORAGE_OK
        );
        let mut canonical_frame = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut canonical_frame,
                )
            },
            YU_STORAGE_OK
        );
        let state = unsafe { raw.as_ref() }.expect("session");
        let publication = state
            .macos_render_host
            .as_ref()
            .and_then(|host| host.builder.last_publication())
            .expect("canonical publication");
        assert!(
            publication
                .frame()
                .scene()
                .scene()
                .primitives()
                .iter()
                .any(|primitive| {
                    matches!(
                        primitive,
                        Primitive::Ornament(table)
                            if table.role() == yu_scene::OrnamentRole::Border
                                && (table.bounds().x() + table.bounds().width() * 0.5 - divider).abs() < 0.01
                    )
                })
        );
        assert_eq!(state.session.snapshot().as_str(), source);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn frame_ticket_checks_request_surface_binding_resources_and_revision_independently() {
        let make_key = |revision| {
            FrameBuildKey::new(
                revision,
                0,
                vec![],
                0,
                None,
                Appearance::Light,
                FrameGeometry::new(16.0, 500.0, 0.0, 240.0, 500.0, 240.0, 2.0)
                    .expect("valid FFI session fixture"),
            )
        };
        let ticket = MacosFrameTicket {
            request: FrameBuildRequest::new(make_key(7), 11),
            surface_generation: 3,
            binding_generation: 4,
            resource_generation: 5,
        };
        assert!(ticket.accepts(&ticket));
        let mut changed = ticket.clone();
        changed.request = FrameBuildRequest::new(make_key(7), 12);
        assert!(
            !ticket.accepts(&changed),
            "A→B→A must not accept the old A ticket"
        );
        let mut changed = ticket.clone();
        changed.surface_generation += 1;
        assert!(
            !ticket.accepts(&changed),
            "same dimensions do not mean same surface"
        );
        let mut changed = ticket.clone();
        changed.binding_generation += 1;
        assert!(
            !ticket.accepts(&changed),
            "detach/rebind must invalidate old work"
        );
        let mut changed = ticket.clone();
        changed.resource_generation += 1;
        assert!(!ticket.accepts(&changed));
        let mut changed = ticket.clone();
        changed.request = FrameBuildRequest::new(make_key(8), 11);
        assert!(!ticket.accepts(&changed));
        assert_eq!(ticket.request.generation(), 11);
        assert_eq!(ticket.resource_generation, 5);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn background_frame_snapshot_uses_accepted_publication() {
        let path = std::env::temp_dir().join(format!("yu-background-host-{}.md", temp_id()));
        fs::write(
            &path,
            format!("# Background frame\n\n{}", "Hello 羽🙂\n\n".repeat(100)),
        )
        .expect("valid FFI session fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let session = unsafe { raw.as_mut() }.expect("valid FFI session fixture");
        let request = MacosFrameRequest {
            expected_revision: 0,
            size: 16.0,
            max_width: 500.0,
            scroll_y: 0.0,
            viewport_height: 240.0,
            surface_width: 500.0,
            surface_height: 240.0,
            surface_generation: 0,
            raster_scale: 2.0,
            appearance: Appearance::Light,
        };
        assert_eq!(
            macos_render_host_frame(session, request, true),
            Err(YU_STORAGE_RENDER_BUSY)
        );
        fn finish(
            session: &mut YuStorageSession,
            request: MacosFrameRequest,
        ) -> YuStorageMacosRenderHostSnapshot {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                match macos_render_host_frame(session, request, true) {
                    Ok(snapshot) => return snapshot,
                    Err(YU_STORAGE_RENDER_BUSY) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    result => {
                        panic!("background publication did not become observable: {result:?}")
                    }
                }
            }
        }
        let snapshot = finish(session, request);
        assert_eq!(
            session
                .session
                .document()
                .editor()
                .layout_cache_stats()
                .builds(),
            0,
            "worker measurements must be adopted without building owner layouts"
        );
        let first_generation = session
            .macos_render_host
            .as_ref()
            .expect("valid FFI session fixture")
            .frame_request_generation;

        let state = session
            .macos_render_host
            .as_ref()
            .expect("valid FFI session fixture");
        assert!(
            state.builder.last_publication().is_none(),
            "must exercise the worker, not synchronous fallback"
        );
        assert_eq!(snapshot.frame_revision, 0);
        assert_eq!(
            snapshot.frame_serial,
            state
                .host
                .frame_serial()
                .expect("valid FFI session fixture")
        );
        assert!(snapshot.command_count > 0);
        assert!(snapshot.atlas_page_count > 0);
        assert!(snapshot.content_height > 0.0);

        // No drawable has been submitted, but a complete CPU frame is available.
        let key = frame_key(
            session,
            Appearance::Light,
            FrameGeometry::new(16.0, 500.0, 0.0, 240.0, 500.0, 240.0, 2.0)
                .expect("valid FFI session fixture"),
        );
        let state = session
            .macos_render_host
            .as_mut()
            .expect("valid FFI session fixture");
        assert!(state.last_frame_key.is_none());
        state.resource_refresh_pending = true;
        let viewport = Rect::new(0.0, 0.0, 500.0, 240.0).expect("valid FFI session fixture");
        let resource_generation = state.resource_completion_generation;
        assert_eq!(
            macos_can_reuse_prepared_frame(state, &key, viewport, 0, resource_generation),
            !state.layout_pending,
            "pending layout requires another bounded batch"
        );
        assert!(!macos_can_reuse_prepared_frame(
            state,
            &key,
            viewport,
            1,
            resource_generation
        ));
        assert!(!macos_can_reuse_prepared_frame(
            state,
            &key,
            viewport,
            0,
            resource_generation.wrapping_add(1)
        ));
        state.resource_retry_pending = true;
        assert!(!macos_can_reuse_prepared_frame(
            state,
            &key,
            viewport,
            0,
            resource_generation
        ));
        state.resource_retry_pending = false;
        state.resource_refresh_pending = false;

        // Supersede a pending same-revision resize on the same worker.
        let resizing = MacosFrameRequest {
            max_width: 520.0,
            surface_width: 520.0,
            surface_generation: 1,
            ..request
        };
        assert_eq!(
            macos_render_host_frame(session, resizing, true),
            Err(YU_STORAGE_RENDER_BUSY)
        );
        let resized = finish(
            session,
            MacosFrameRequest {
                max_width: 540.0,
                surface_width: 540.0,
                surface_generation: 2,
                ..request
            },
        );
        assert!(resized.frame_serial > snapshot.frame_serial);
        assert_eq!(resized.surface_generation, 2);
        assert_eq!(resized.viewport_width, 540.0);

        // Replacing a surface without changing geometry must still supersede
        // preparation. Neither request is allowed to alter the visible frame
        // before its new ticket has been accepted.
        let same_geometry = MacosFrameRequest {
            max_width: 540.0,
            surface_width: 540.0,
            surface_generation: 3,
            ..request
        };
        assert_eq!(
            macos_render_host_frame(session, same_geometry, true),
            Err(YU_STORAGE_RENDER_BUSY)
        );
        let pending_generation = session
            .macos_render_host
            .as_ref()
            .expect("valid FFI session fixture")
            .frame_request_generation;
        let replacement = MacosFrameRequest {
            surface_generation: 4,
            ..same_geometry
        };
        assert_eq!(
            macos_render_host_frame(session, replacement, true),
            Err(YU_STORAGE_RENDER_BUSY)
        );
        let state = session
            .macos_render_host
            .as_ref()
            .expect("valid FFI session fixture");
        assert!(state.frame_request_generation > pending_generation);
        assert_eq!(
            state
                .last_publication
                .as_ref()
                .expect("valid FFI session fixture")
                .serial(),
            resized.frame_serial
        );
        assert_eq!(state.host.frame_serial(), Some(resized.frame_serial));
        let replaced = finish(session, replacement);
        assert_eq!(replaced.surface_generation, 4);
        assert!(replaced.frame_serial > resized.frame_serial);

        // A new attachment starts at generation zero, independent of the old resize history.
        assert_eq!(
            unsafe { yu_storage_session_macos_render_host_surface_detach(raw) },
            YU_STORAGE_OK
        );
        let session = unsafe { raw.as_mut() }.expect("valid FFI session fixture");
        assert!(
            session
                .macos_render_host
                .as_ref()
                .expect("valid FFI session fixture")
                .frame_worker_request
                .is_none()
        );
        let rebound = finish(session, request);
        assert!(rebound.frame_serial > replaced.frame_serial);
        assert_eq!(rebound.surface_generation, 0);
        assert!(
            session
                .macos_render_host
                .as_ref()
                .expect("valid FFI session fixture")
                .frame_request_generation
                > first_generation
        );
        assert_eq!(
            session
                .macos_render_host
                .as_ref()
                .expect("valid FFI session fixture")
                .binding_generation,
            1
        );

        // Scroll is deliberately absent from FrameBuildKey. A pending result
        // still must cover the latest camera before it may replace the frame.
        assert_eq!(
            macos_render_host_frame(session, request, true),
            Err(YU_STORAGE_RENDER_BUSY)
        );
        let scrolled = finish(
            session,
            MacosFrameRequest {
                scroll_y: 1500.0,
                ..request
            },
        );
        let frame = session
            .macos_render_host
            .as_ref()
            .expect("valid FFI session fixture")
            .host
            .frame_handle()
            .expect("valid FFI session fixture");
        assert_eq!(scrolled.scroll_y, 1500.0);
        assert_eq!(frame.plan().viewport().y(), 1500.0);
        assert!(macos_frame_covers_viewport(frame.as_ref(), 1500.0, 240.0));

        // Diagnostic (1x) and production (2x) builders can alternate without
        // serial regression or adopting glyph pages from the wrong scale.
        let diagnostic = macos_render_host_frame(
            session,
            MacosFrameRequest {
                raster_scale: 1.0,
                ..request
            },
            false,
        )
        .expect("valid FFI session fixture");
        assert!(diagnostic.frame_serial > scrolled.frame_serial);
        let resumed = finish(session, request);
        assert!(resumed.frame_serial > diagnostic.frame_serial);
        let mut progressive = resumed;
        for _ in 0..1000 {
            if progressive.layout_pending == 0 {
                break;
            }
            let next = finish(session, request);
            assert!(next.frame_serial > progressive.frame_serial);
            progressive = next;
        }
        assert_eq!(
            progressive.layout_pending, 0,
            "bounded background batches must converge"
        );

        for enabled in [1, 0, 0] {
            assert_eq!(
                unsafe { yu_storage_session_set_source_mode(session, enabled) },
                YU_STORAGE_OK
            );
            assert!(
                session
                    .macos_render_host
                    .as_ref()
                    .expect("host")
                    .frame_worker
                    .is_some(),
                "mode switch must retain its worker"
            );
            let mut frame = finish(session, request);
            for _ in 0..1000 {
                if frame.layout_pending == 0 {
                    break;
                }
                frame = finish(session, request);
            }
            assert_eq!(frame.layout_pending, 0, "mode switch must converge");
        }
        let before_trim = session
            .macos_render_host
            .as_ref()
            .expect("host")
            .last_publication
            .as_ref()
            .expect("frame")
            .serial();
        assert_eq!(
            unsafe { yu_storage_session_trim_render_caches(session) },
            YU_STORAGE_OK
        );
        assert!(
            session
                .macos_render_host
                .as_ref()
                .expect("host")
                .frame_worker
                .is_some(),
            "memory pressure must not disable future background work"
        );
        let after_trim = finish(session, request);
        assert!(
            after_trim.frame_serial > before_trim,
            "cache trim must preserve publication ordering"
        );

        assert_eq!(
            session
                .macos_render_host
                .as_ref()
                .expect("valid FFI session fixture")
                .builder
                .config()
                .raster_scale(),
            2.0
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("valid FFI session fixture");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_render_host_reuses_state_across_viewport_events() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-host-{id}.md"));
        fs::write(&path, "# 羽🙂\n\nparagraph 日本語 **bold**\n\n- [ ] task\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut first = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut first,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(first.revision, 0);
        assert_eq!(first.frame_revision, 0);
        assert_eq!(first.surface_generation, 0);
        assert_eq!(first.frame_serial, 1);
        assert!(first.command_count > 0);
        assert!(first.upload_count > 0);
        assert!(first.damage_count > 0);
        assert!(first.atlas_page_count > 0);
        assert!(first.atlas_glyph_count > 0);
        assert!(first.published != 0);
        assert_eq!(first.selection_decoration_count, 0);
        assert_eq!(first.caret_decoration_count, 1);

        let mut repeated = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut repeated,
                )
            },
            YU_STORAGE_OK
        );
        assert!(repeated.frame_serial > first.frame_serial);
        assert_eq!(repeated.atlas_page_count, first.atlas_page_count);
        assert_eq!(repeated.atlas_glyph_count, first.atlas_glyph_count);
        assert_eq!(repeated.upload_count, 0);

        let source = "# 羽🙂\n\nparagraph 日本語 **bold**\n\n- [ ] task\n";
        let composition_start = source.find("日本語").expect("composition start");
        let composition_end = source.find("task").expect("composition end") + "ta".len();
        let composition_start_utf16 = source[..composition_start].encode_utf16().count() as u64;
        let composition_end_utf16 = source[..composition_end].encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_begin_composition(
                    raw,
                    0,
                    composition_start_utf16,
                    composition_end_utf16,
                    "日本🙂".as_ptr(),
                    "日本🙂".len(),
                    2,
                    2,
                )
            },
            YU_STORAGE_OK
        );

        let mut cross_block = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut cross_block,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(cross_block.revision, 0);
        assert_eq!(cross_block.composition_generation, 1);
        assert!(cross_block.frame_serial > repeated.frame_serial);
        assert!(cross_block.command_count > 0);
        assert!(cross_block.atlas_glyph_count >= repeated.atlas_glyph_count);

        assert_eq!(
            unsafe { yu_storage_session_cancel_composition(raw, 0, 1) },
            YU_STORAGE_OK
        );
        let mut after_cancel = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut after_cancel,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(after_cancel.revision, 0);
        assert_eq!(after_cancel.composition_generation, 2);
        assert_eq!(after_cancel.command_count, repeated.command_count);
        assert!(after_cancel.frame_serial > cross_block.frame_serial);

        let mut resized = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    12.0,
                    180.0,
                    1,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut resized,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(resized.surface_generation, 1);
        assert_eq!(resized.scroll_y, 12.0);
        assert_eq!(resized.viewport_height, 180.0);
        assert!(resized.frame_serial > repeated.frame_serial);

        let mut regressed_generation = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    0,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut regressed_generation,
                )
            },
            YU_STORAGE_RENDER_HOST_UNAVAILABLE
        );
        assert_eq!(
            regressed_generation,
            YuStorageMacosRenderHostSnapshot::default()
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        let mut stale = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    1,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut stale,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(stale, YuStorageMacosRenderHostSnapshot::default());

        let mut next = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw,
                    1,
                    14.0,
                    500.0,
                    0.0,
                    240.0,
                    1,
                    YU_STORAGE_APPEARANCE_LIGHT,
                    &mut next,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(next.revision, 1);
        assert_eq!(next.frame_revision, 1);
        assert_eq!(next.surface_generation, 1);
        assert!(next.frame_serial > resized.frame_serial);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_shaped_caret_scroll_request_is_revision_bound_and_document_space() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-caret-scroll-{id}.md"));
        let source =
            "# one\n\nparagraph one\n\n# two\n\nparagraph two\n\n# three\n\nparagraph three\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        // 这些断言用行高做几何基准；配置的发布已经由 Rust 自己完成，
        // 这里只是取同一份度量来算期望值。
        let (_, metrics, _) =
            core_text_layout(14.0, 500.0, yu_core::ThemeId::Github).expect("CoreText");

        let source_utf16 = source.encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    source_utf16,
                    source_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        // 余量现在由 Rust 取一个行高，视口必须容得下它，否则测的就不再是
        // 「光标在视口外要滚多少」而是余量本身的退化情形。
        let viewport_height = metrics.line_height() * 4.0;
        let mut request = YuStorageCaretScrollRequest::default();
        assert_eq!(
            unsafe {
                yu_storage_session_shaped_caret_scroll_request(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    viewport_height,
                    &mut request,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(request.revision, 0);
        assert_eq!(request.source_utf16, source_utf16);
        assert!(request.block_index > 0);
        assert!(request.caret_y.is_finite() && request.caret_y > 0.0);
        assert_eq!(request.current_scroll_y, 0.0);
        assert!(request.target_scroll_y > 0.0);
        assert_eq!(request.needs_scroll, 1);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"x".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_shaped_caret_scroll_request(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    viewport_height,
                    &mut request,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(request, YuStorageCaretScrollRequest::default());

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_composition_shaped_caret_is_generation_bound() {
        let id = temp_id();
        let path =
            std::env::temp_dir().join(format!("yu-storage-ffi-composition-shaped-caret-{id}.md"));
        let source = "before **x** after";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_begin_composition(
                    raw,
                    0,
                    9,
                    10,
                    "日本🙂".as_ptr(),
                    "日本🙂".len(),
                    2,
                    4,
                )
            },
            YU_STORAGE_OK
        );

        let mut caret = YuStorageCompositionShapedCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_composition_shaped_caret(
                    raw,
                    0,
                    1,
                    9,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    14.0,
                    500.0,
                    &mut caret,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(caret.revision, 0);
        assert_eq!(caret.generation, 1);
        assert_eq!(caret.source_utf16, 9);
        assert_eq!(caret.block_index, 0);
        assert_eq!(caret.visual_utf16, 11);
        assert_eq!(caret.round_trip_source_utf16, 10);
        assert_eq!(caret.visual_selection_start_utf16, 9);
        assert_eq!(caret.visual_selection_end_utf16, 11);
        assert_eq!(caret.visual_replacement_start_utf16, 7);
        assert_eq!(caret.visual_replacement_end_utf16, 11);
        assert!(caret.caret_x.is_finite());
        assert!(caret.caret_y.is_finite());
        assert!(caret.caret_height > 0.0);

        let mut updated = YuStorageCompositionShapedCaret {
            revision: 99,
            ..YuStorageCompositionShapedCaret::default()
        };
        assert_eq!(
            unsafe {
                yu_storage_session_composition_shaped_caret(
                    raw,
                    0,
                    2,
                    9,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    14.0,
                    500.0,
                    &mut updated,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        assert_eq!(updated, YuStorageCompositionShapedCaret::default());

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_accessibility_snapshot_and_line_ranges_are_revision_bound() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-ax-{id}.md"));
        fs::write(&path, "# 羽\n日本語 🙂\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut snapshot = YuStorageAccessibilitySnapshot::default();
        assert_eq!(
            unsafe { yu_storage_session_accessibility_snapshot(raw, &mut snapshot) },
            YU_STORAGE_OK
        );
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.number_of_characters_utf16, 11);
        assert_eq!(snapshot.selection_start_utf16, 0, "新文档的光标落在文首");
        assert_eq!(snapshot.selection_end_utf16, 0);
        assert_eq!(snapshot.line_count, 3);
        assert_eq!(
            snapshot.selection_affinity,
            YU_STORAGE_CARET_AFFINITY_DOWNSTREAM
        );

        let mut first_line = YuStorageAccessibilityRange::default();
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_range(
                    raw,
                    snapshot.revision,
                    0,
                    &mut first_line,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(first_line.revision, snapshot.revision);
        assert_eq!(first_line.start_utf16, 0);
        assert_eq!(first_line.end_utf16, 4);

        let mut third_line = YuStorageAccessibilityRange::default();
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_range(
                    raw,
                    snapshot.revision,
                    2,
                    &mut third_line,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(third_line.start_utf16, 11);
        assert_eq!(third_line.end_utf16, 11);

        let mut line = u64::MAX;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_for_position(
                    raw,
                    snapshot.revision,
                    5,
                    &mut line,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(line, 1);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, snapshot.revision, "x".as_ptr(), 1, &mut result)
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_line_range(
                    raw,
                    snapshot.revision,
                    0,
                    &mut first_line,
                )
            },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_accessibility_labels_resolve_html_without_mutating_source() {
        let source = "<details><summary>Hi <em>中文</em> &amp; <strong>friends</strong><br><img src='x.png' alt='图 &amp; 字'> 😀</summary><p>Secret</p></details>\n\n<widget>Keep &amp;</widget>";
        let path = std::env::temp_dir().join(format!("yu-ax-label-{}.md", temp_id()));
        fs::write(&path, source).expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let start = source.find("Hi ").expect("summary");
        let end = source.find("</summary>").expect("summary end");
        let start_utf16 = source[..start].encode_utf16().count() as u64;
        let end_utf16 = source[..end].encode_utf16().count() as u64;
        let initial_selection = unsafe { &*raw }.session.selection();
        let mut needed = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_accessibility_label(
                    raw,
                    0,
                    start_utf16,
                    end_utf16,
                    ptr::null_mut(),
                    0,
                    &mut needed,
                )
            },
            YU_STORAGE_OK
        );
        let mut output = vec![0; needed];
        assert_eq!(
            unsafe {
                yu_storage_session_copy_accessibility_label(
                    raw,
                    0,
                    start_utf16,
                    end_utf16,
                    output.as_mut_ptr(),
                    output.len(),
                    &mut needed,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            String::from_utf8(output).expect("UTF8"),
            "Hi 中文 & friends 图 & 字 😀"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_copy_accessibility_label(
                    raw,
                    1,
                    start_utf16,
                    end_utf16,
                    ptr::null_mut(),
                    0,
                    &mut needed,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        let emoji = source[..source.find('😀').expect("emoji")]
            .encode_utf16()
            .count() as u64;
        assert_ne!(
            unsafe {
                yu_storage_session_copy_accessibility_label(
                    raw,
                    0,
                    emoji + 1,
                    emoji + 2,
                    ptr::null_mut(),
                    0,
                    &mut needed,
                )
            },
            YU_STORAGE_OK
        );
        let alternate = source.find("图 &amp; 字").expect("alternate text");
        let alternate_start = source[..alternate].encode_utf16().count() as u64;
        let mut attribute = vec![0; 64];
        assert_eq!(
            unsafe {
                yu_storage_session_copy_accessibility_label(
                    raw,
                    0,
                    alternate_start,
                    alternate_start + "图 &amp; 字".encode_utf16().count() as u64,
                    attribute.as_mut_ptr(),
                    attribute.len(),
                    &mut needed,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(&attribute[..needed], "图 &amp; 字".as_bytes());
        let unknown = source.find("<widget>").expect("unknown");
        let mut output = vec![0; 128];
        assert_eq!(
            unsafe {
                yu_storage_session_copy_accessibility_label(
                    raw,
                    0,
                    source[..unknown].encode_utf16().count() as u64,
                    source.encode_utf16().count() as u64,
                    output.as_mut_ptr(),
                    output.len(),
                    &mut needed,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(&output[..needed], b"<widget>Keep &amp;</widget>");
        assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), source);
        assert_eq!(unsafe { &*raw }.session.selection(), initial_selection);
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_accessibility_semantic_nodes_are_revision_bound_and_source_backed() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-semantic-ax-{id}.md"));
        fs::write(
            &path,
            "# 标题\n\n段落 **粗体** [链接](https://example.com) [参考][rust]\n\n- [x] 完成\n\n[rust]: https://www.rust-lang.org/\n",
        )
        .expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_semantic_nodes_v2(
                    raw,
                    0,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            YU_STORAGE_OK
        );
        assert!(
            count >= 6,
            "root, blocks, and inline semantic nodes (count={count})"
        );

        let mut nodes = vec![YuStorageAccessibilityNodeV2::default(); count];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_semantic_nodes_v2(
                    raw,
                    0,
                    nodes.as_mut_ptr(),
                    nodes.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, count);
        assert_eq!(nodes[0].revision, 0);
        assert_eq!(nodes[0].index, 0);
        assert_eq!(nodes[0].parent, YU_STORAGE_ACCESSIBILITY_PARENT_NONE);
        assert_eq!(nodes[0].kind, YU_STORAGE_ACCESSIBILITY_KIND_DOCUMENT);

        let heading = nodes
            .iter()
            .find(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_HEADING)
            .expect("heading semantic node");
        assert_eq!(heading.parent, 0);
        assert!(heading.label_end_utf16 > heading.label_start_utf16);
        assert!(
            nodes
                .iter()
                .any(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_STRONG)
        );
        assert!(
            nodes
                .iter()
                .any(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_LINK)
        );
        let link = nodes
            .iter()
            .find(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_LINK)
            .expect("link semantic node");
        assert!(link.destination_end_utf16 > link.destination_start_utf16);
        let reference_link = nodes
            .iter()
            .find(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_REFERENCE_LINK)
            .expect("reference link semantic node");
        assert!(reference_link.destination_end_utf16 > reference_link.destination_start_utf16);
        let task = nodes
            .iter()
            .find(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_TASK_LIST_ITEM)
            .expect("task semantic node");
        assert_ne!(task.flags & YU_STORAGE_ACCESSIBILITY_FLAG_TASK_DONE, 0);
        assert_ne!(task.action_block, YU_STORAGE_ACCESSIBILITY_NO_ACTION_BLOCK);

        let mut stale_count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_semantic_nodes_v2(
                    raw,
                    1,
                    ptr::null_mut(),
                    0,
                    &mut stale_count,
                )
            },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// 拉一版大纲跨 C ABI：两遍协议的两个缓冲区一起、层级、面板上那一行
    /// 文字、身份。
    ///
    /// 语料里三样都是特意放的：`## 收尾 ##` 的收尾串由**树**剥掉、`**粗**`
    /// 由**装饰**剥掉、`🙂` 让字节偏移与 UTF-16 偏移给出两组不同的数字。
    #[test]
    fn ffi_outline_items_carry_the_hierarchy_and_the_panel_line() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-outline-{id}.md"));
        let source = "# 🙂 **粗** 一级\n\n段落\n\n## 收尾 ##\n\nSetext\n===\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        // 第一遍：只问两个长度。两个指针都是 null、两个容量都是 0。
        let mut count = 0;
        let mut text_length = 0;
        let mut run_count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_outline_items(
                    raw,
                    0,
                    ptr::null_mut(),
                    0,
                    &mut count,
                    ptr::null_mut(),
                    0,
                    &mut text_length,
                    ptr::null_mut(),
                    0,
                    &mut run_count,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(count, 3, "两条 ATX 加一条 Setext");
        assert!(text_length > 0, "每一条都有 label 与 identity");

        // 条目数组小了要明确拒绝，不能拷半份出去。
        let mut runs = vec![YuStorageOutlineStyleRun::default(); run_count];
        let mut short = vec![YuStorageOutlineItem::default(); count - 1];
        let mut short_text = vec![0u8; text_length];
        let mut short_written = 0;
        let mut short_text_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_outline_items(
                    raw,
                    0,
                    short.as_mut_ptr(),
                    short.len(),
                    &mut short_written,
                    short_text.as_mut_ptr(),
                    short_text.len(),
                    &mut short_text_written,
                    runs.as_mut_ptr(),
                    runs.len(),
                    &mut run_count,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );

        // **文本缓冲小了也要拒绝。** 两个容量各挡各的：只检查条目那一个的话，
        // 文本会被拷成半截 UTF-8，面板上那一行显示成乱码——不报错。
        let mut items = vec![YuStorageOutlineItem::default(); count];
        let mut runs = vec![YuStorageOutlineStyleRun::default(); run_count];
        let mut tiny_text = vec![0u8; text_length - 1];
        let mut written = 0;
        let mut tiny_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_outline_items(
                    raw,
                    0,
                    items.as_mut_ptr(),
                    items.len(),
                    &mut written,
                    tiny_text.as_mut_ptr(),
                    tiny_text.len(),
                    &mut tiny_written,
                    runs.as_mut_ptr(),
                    runs.len(),
                    &mut run_count,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );

        // A short style buffer must not modify either of the other payloads.
        let mut guard_text = vec![77u8; text_length];
        let original_items = items.clone();
        let mut short_styles = vec![YuStorageOutlineStyleRun::default(); run_count - 1];
        let mut style_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_outline_items(
                    raw,
                    0,
                    items.as_mut_ptr(),
                    items.len(),
                    &mut written,
                    guard_text.as_mut_ptr(),
                    guard_text.len(),
                    &mut tiny_written,
                    short_styles.as_mut_ptr(),
                    short_styles.len(),
                    &mut style_written,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );
        assert_eq!(items, original_items);
        assert!(guard_text.iter().all(|byte| *byte == 77));
        assert!(
            short_styles
                .iter()
                .all(|run| *run == YuStorageOutlineStyleRun::default())
        );
        assert_eq!(style_written, run_count);
        // A mixed measure/write request is invalid, even with enough style capacity.
        assert_eq!(
            unsafe {
                yu_storage_session_outline_items(
                    raw,
                    0,
                    ptr::null_mut(),
                    0,
                    &mut written,
                    ptr::null_mut(),
                    0,
                    &mut tiny_written,
                    runs.as_mut_ptr(),
                    runs.len(),
                    &mut style_written,
                )
            },
            YU_STORAGE_NULL_POINTER
        );

        // 第二遍：拷出来。
        let read = |revision: u64| -> Result<(Vec<YuStorageOutlineItem>, String), i32> {
            let mut count = 0;
            let mut text_length = 0;
            let mut run_count = 0;
            let status = unsafe {
                yu_storage_session_outline_items(
                    raw,
                    revision,
                    ptr::null_mut(),
                    0,
                    &mut count,
                    ptr::null_mut(),
                    0,
                    &mut text_length,
                    ptr::null_mut(),
                    0,
                    &mut run_count,
                )
            };
            if status != YU_STORAGE_OK {
                return Err(status);
            }
            let mut items = vec![YuStorageOutlineItem::default(); count];
            let mut runs = vec![YuStorageOutlineStyleRun::default(); run_count];
            let mut text = vec![0u8; text_length];
            let mut written = 0;
            let mut text_written = 0;
            let status = unsafe {
                yu_storage_session_outline_items(
                    raw,
                    revision,
                    items.as_mut_ptr(),
                    items.len(),
                    &mut written,
                    text.as_mut_ptr(),
                    text.len(),
                    &mut text_written,
                    runs.as_mut_ptr(),
                    runs.len(),
                    &mut run_count,
                )
            };
            if status != YU_STORAGE_OK {
                return Err(status);
            }
            assert_eq!(written, count, "两遍给出的条数必须一致");
            assert_eq!(text_written, text_length, "两遍给出的文本长度必须一致");
            if revision == 0 {
                let first = &items[0];
                let slice = &runs[first.style_offset as usize
                    ..(first.style_offset + first.style_count) as usize];
                assert!(
                    slice
                        .iter()
                        .any(|run| run.start_utf16 == 3 && run.end_utf16 == 4 && run.traits == 1)
                );
            }
            Ok((items, String::from_utf8(text).expect("文本是合法 UTF-8")))
        };

        let (items, text) = read(0).expect("拷出来");
        let piece = |offset: u64, length: u64| {
            let start = usize::try_from(offset).expect("偏移放得进 usize");
            let end = start + usize::try_from(length).expect("长度放得进 usize");
            text[start..end].to_owned()
        };
        let display =
            |item: &YuStorageOutlineItem| piece(item.display_utf8_offset, item.display_utf8_length);
        let identity = |item: &YuStorageOutlineItem| {
            piece(item.identity_utf8_offset, item.identity_utf8_length)
        };
        let utf16: Vec<u16> = source.encode_utf16().collect();
        let label_source = |item: &YuStorageOutlineItem| {
            let start = usize::try_from(item.label_start_utf16).expect("UTF-16 偏移放得进 usize");
            let end = usize::try_from(item.label_end_utf16).expect("UTF-16 偏移放得进 usize");
            String::from_utf16(&utf16[start..end]).expect("正文是合法 UTF-16")
        };

        assert_eq!(items[0].revision, 0);
        assert_eq!(items[0].index, 0);
        assert_eq!(items[0].parent, YU_STORAGE_OUTLINE_PARENT_NONE);
        assert_eq!(items[0].level, 1);
        assert_eq!(items[0].child_count, 1, "二级标题是它的直接孩子");
        assert_eq!(
            label_source(&items[0]),
            "🙂 **粗** 一级",
            "正文区间不剥装饰"
        );
        assert_eq!(display(&items[0]), "🙂 粗 一级", "面板上那一行剥掉了 `**`");

        assert_eq!(items[1].parent, 0, "二级标题挂在一级下");
        assert_eq!(items[1].level, 2);
        assert_eq!(items[1].child_count, 0);
        assert_eq!(display(&items[1]), "收尾", "收尾的 `#` 串不是正文");

        assert_eq!(items[2].parent, YU_STORAGE_OUTLINE_PARENT_NONE);
        assert_eq!(items[2].child_count, 0);
        assert_eq!(display(&items[2]), "Setext", "下划线那一行不是正文");

        // 身份两两不同，而且每一条都不是空的——空身份会让展开状态整片合并。
        let identities: Vec<String> = items.iter().map(identity).collect();
        assert!(identities.iter().all(|value| !value.is_empty()));
        assert_eq!(
            identities
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            identities.len(),
            "{identities:?}"
        );

        // source 包着 label。
        for item in &items {
            assert!(item.source_start_utf16 <= item.label_start_utf16);
            assert!(item.label_end_utf16 <= item.source_end_utf16);
        }

        // **把光标放进 `**粗**` 里**——编辑器里那两个 `**` 会露出来，但面板上
        // 的标签不能跟着变。这条压的是「走的是规范装饰那条路」。
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    7,
                    7,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let (revealed, revealed_text) = read(0).expect("光标在标题里");
        let revealed_display = {
            let start = usize::try_from(revealed[0].display_utf8_offset).expect("偏移放得进 usize");
            let end =
                start + usize::try_from(revealed[0].display_utf8_length).expect("长度放得进 usize");
            revealed_text[start..end].to_owned()
        };
        assert_eq!(
            revealed_display, "🙂 粗 一级",
            "光标露出不能影响面板上的标签"
        );

        assert_eq!(read(1), Err(YU_STORAGE_STALE_REVISION));

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    /// 搜索跨 C ABI：两遍协议、UTF-16、结果那一行显示成什么、编辑之后重扫。
    ///
    /// 「匹配找得对不对」在 `yu-editor::search` 的用例里，「那一行剥干净没有」
    /// 在 `yu-editor` 的 `panel` 用例里；这里压的是 ABI 那一层：位置换算与
    /// 两个缓冲区的对齐。
    #[test]
    fn ffi_search_matches_carry_utf16_and_the_panel_line() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-search-{id}.md"));
        // 🙂 让字节偏移与 UTF-16 偏移分开：`目标` 在字节里从 9 起，在 UTF-16
        // 里从 5 起。
        let source = "# 🙂 目标\n\n段落里也有**目标**。\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let matches = |revision: u64| -> Result<(Vec<YuStorageSearchMatch>, String), i32> {
            let mut count = 0;
            let mut text_length = 0;
            let status = unsafe {
                yu_storage_session_search_matches(
                    raw,
                    revision,
                    ptr::null_mut(),
                    0,
                    &mut count,
                    ptr::null_mut(),
                    0,
                    &mut text_length,
                )
            };
            if status != YU_STORAGE_OK {
                return Err(status);
            }
            let mut values = vec![YuStorageSearchMatch::default(); count];
            let mut text = vec![0u8; text_length];
            let mut written = 0;
            let mut text_written = 0;
            let status = unsafe {
                yu_storage_session_search_matches(
                    raw,
                    revision,
                    values.as_mut_ptr(),
                    values.len(),
                    &mut written,
                    text.as_mut_ptr(),
                    text.len(),
                    &mut text_written,
                )
            };
            if status != YU_STORAGE_OK {
                return Err(status);
            }
            assert_eq!(written, count, "两遍给出的条数必须一致");
            assert_eq!(text_written, text_length, "两遍给出的文本长度必须一致");
            Ok((values, String::from_utf8(text).expect("文本是合法 UTF-8")))
        };

        // 还没有查询：0 条，不是错误。
        assert_eq!(matches(0).expect("没有查询"), (vec![], String::new()));

        let query = "目标".as_bytes();
        assert_eq!(
            unsafe { yu_storage_session_set_search_query(raw, query.as_ptr(), query.len()) },
            YU_STORAGE_OK
        );
        let (hits, text) = matches(0).expect("两处命中");
        assert_eq!(hits.len(), 2);
        let display = |hit: &YuStorageSearchMatch| {
            let start = usize::try_from(hit.display_utf8_offset).expect("偏移放得进 usize");
            let end = start + usize::try_from(hit.display_utf8_length).expect("长度放得进 usize");
            text[start..end].to_owned()
        };

        // 位置是 UTF-16：标题里的 `目标` 从 5 起（字节是 9）。
        assert_eq!((hits[0].start_utf16, hits[0].end_utf16), (5, 7));
        assert_eq!(display(&hits[0]), "🙂 目标", "标题那一行");
        // 第二处在段落里，行尾的换行被收掉，`**` 被剥掉。
        assert!(hits[1].start_utf16 > hits[0].end_utf16);
        assert_eq!(display(&hits[1]), "段落里也有目标。");

        // 条目数组小了要拒绝，不能拷半份出去。
        let mut short = vec![YuStorageSearchMatch::default(); 1];
        let mut short_text = vec![0u8; 1024];
        let mut short_written = 0;
        let mut short_text_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_search_matches(
                    raw,
                    0,
                    short.as_mut_ptr(),
                    short.len(),
                    &mut short_written,
                    short_text.as_mut_ptr(),
                    short_text.len(),
                    &mut short_text_written,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );

        // 编辑之后必须重扫：在文首插入一个字符会把每一处命中都推后。
        let insert = "X".as_bytes();
        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    0,
                    0,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, 0, insert.as_ptr(), insert.len(), &mut result)
            },
            YU_STORAGE_OK
        );
        assert_eq!(matches(0), Err(YU_STORAGE_STALE_REVISION), "旧 Revision");
        let (shifted, _) = matches(result.revision).expect("编辑之后");
        assert_eq!(shifted.len(), 2, "编辑没有丢掉命中");
        assert_eq!(
            (shifted[0].start_utf16, shifted[0].end_utf16),
            (6, 8),
            "插入一个字符必须把命中整体推后一位——不重扫的话它会停在旧位置"
        );

        // 收掉搜索。
        assert_eq!(
            unsafe { yu_storage_session_set_search_query(raw, ptr::null(), 0) },
            YU_STORAGE_OK
        );
        assert_eq!(
            matches(result.revision).expect("收掉之后"),
            (vec![], String::new())
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn revision_query_survives_disk_failure_and_preserves_save_conflicts() {
        let path = std::env::temp_dir().join(format!("yu-revision-query-{}.md", temp_id()));
        fs::write(&path, "original").expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let mut revision = 99;
        assert_eq!(
            unsafe { yu_storage_session_revision(raw, &mut revision) },
            YU_STORAGE_OK
        );
        assert_eq!(revision, 0);
        let mut command = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, revision, b"x".as_ptr(), 1, &mut command)
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_revision(raw, &mut revision) },
            YU_STORAGE_OK
        );
        assert_eq!(revision, command.revision);
        assert_eq!(revision, 1);

        // A directory in place of the file makes full disk inspection fail.
        // Revision must still report the live edited document, never a cache.
        fs::remove_file(&path).expect("remove fixture");
        fs::create_dir(&path).expect("unreadable as document");
        let mut state = YuStorageState::default();
        assert_ne!(
            unsafe { yu_storage_session_state(raw, &mut state) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_revision(raw, &mut revision) },
            YU_STORAGE_OK
        );
        assert_eq!(revision, 1);
        fs::remove_dir(&path).expect("remove temporary directory");
        fs::write(&path, "external change").expect("external edit");
        assert_eq!(
            unsafe { yu_storage_session_state(raw, &mut state) },
            YU_STORAGE_OK
        );
        assert_eq!(state.disk_state, YU_STORAGE_DISK_CHANGED);
        let mut written = 0;
        let mut changed = 0;
        assert_eq!(
            unsafe { yu_storage_session_save(raw, &mut revision, &mut written, &mut changed) },
            YU_STORAGE_EXTERNAL_CHANGE
        );
        assert_eq!(
            fs::read_to_string(&path).expect("external content"),
            "external change"
        );
        assert_eq!(
            unsafe { yu_storage_session_revision(raw, ptr::null_mut()) },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(
            unsafe { yu_storage_session_revision(ptr::null(), &mut revision) },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(revision, 0);
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_word_deletion_round_trips_source_and_history() {
        for (command, at, expected) in [
            (YU_STORAGE_COMMAND_DELETE_WORD_BACKWARD, 5, " beta"),
            (YU_STORAGE_COMMAND_DELETE_WORD_FORWARD, 6, "alpha "),
        ] {
            let path = std::env::temp_dir().join(format!("yu-word-delete-{}.md", temp_id()));
            fs::write(&path, "alpha beta").expect("fixture");
            let path_bytes = path.to_string_lossy().as_bytes().to_vec();
            let mut raw = ptr::null_mut();
            assert_eq!(
                unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
                YU_STORAGE_OK
            );
            assert_eq!(
                unsafe {
                    yu_storage_session_set_selection_endpoints(
                        raw,
                        0,
                        at,
                        at,
                        YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    )
                },
                YU_STORAGE_OK
            );
            let mut result = YuStorageCommandResult::default();
            assert_eq!(
                unsafe { yu_storage_session_execute_command(raw, command, 0, &mut result) },
                YU_STORAGE_OK
            );
            assert_eq!(result.revision, 1);
            assert_eq!(
                unsafe { &*raw }
                    .session
                    .document()
                    .editor()
                    .snapshot()
                    .as_str(),
                expected
            );
            assert_eq!(
                unsafe {
                    yu_storage_session_execute_command(raw, YU_STORAGE_COMMAND_UNDO, 0, &mut result)
                },
                YU_STORAGE_OK
            );
            assert_eq!(
                unsafe { &*raw }
                    .session
                    .document()
                    .editor()
                    .snapshot()
                    .as_str(),
                "alpha beta"
            );
            unsafe { yu_storage_session_destroy(raw) };
            fs::remove_file(path).expect("cleanup");
        }
    }

    #[test]
    fn unified_ffi_command_and_composition_share_revision() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-editor-{id}.md"));
        fs::write(&path, "输入: ").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert!(!raw.is_null());

        let mut selection = YuStorageSelectionEndpoints::default();
        assert_eq!(
            unsafe { yu_storage_session_selection_endpoints(raw, &mut selection) },
            YU_STORAGE_OK
        );
        assert_eq!(selection.revision, 0);
        assert_eq!(selection.focus_utf16, 0, "新文档的光标落在文首");
        // 后面的 MOVE_LEFT 与 composition 都以「光标在末尾」为前提，显式建立。
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    4,
                    4,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(
                    raw,
                    YU_STORAGE_COMMAND_MOVE_LEFT,
                    0,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(result.revision, 0);
        assert_eq!(result.selection_end_utf16, 3);

        let preedit = "にほんご";
        assert_eq!(
            unsafe {
                yu_storage_session_begin_composition(
                    raw,
                    0,
                    4,
                    4,
                    preedit.as_ptr(),
                    preedit.len(),
                    4,
                    4,
                )
            },
            YU_STORAGE_OK
        );
        let mut state = YuStorageState::default();
        assert_eq!(
            unsafe { yu_storage_session_state(raw, &mut state) },
            YU_STORAGE_OK
        );
        assert_eq!(state.revision, 0);
        assert_eq!(state.dirty, 0);
        let mut composition = YuStorageCompositionState::default();
        assert_eq!(
            unsafe { yu_storage_session_composition(raw, &mut composition) },
            YU_STORAGE_OK
        );
        assert_eq!(composition.active, 1);
        assert_eq!(composition.generation, 1);
        let mut preedit_bytes = vec![0_u8; composition.preedit_utf8_length as usize];
        let mut preedit_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_composition(
                    raw,
                    composition.revision,
                    composition.generation,
                    preedit_bytes.as_mut_ptr(),
                    preedit_bytes.len(),
                    &mut preedit_written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(preedit_written, preedit_bytes.len());
        assert_eq!(String::from_utf8(preedit_bytes).expect("preedit"), preedit);
        assert_eq!(
            unsafe {
                yu_storage_session_update_composition(
                    raw,
                    composition.revision,
                    composition.generation.saturating_sub(1),
                    preedit.as_ptr(),
                    preedit.len(),
                    4,
                    4,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        let mut after_stale = YuStorageCompositionState::default();
        assert_eq!(
            unsafe { yu_storage_session_composition(raw, &mut after_stale) },
            YU_STORAGE_OK
        );
        assert_eq!(after_stale.generation, composition.generation);
        assert_eq!(
            after_stale.preedit_utf8_length,
            composition.preedit_utf8_length
        );
        assert_eq!(
            unsafe {
                yu_storage_session_commit_composition(
                    raw,
                    composition.revision,
                    composition.generation,
                    "日本語".as_ptr(),
                    "日本語".len(),
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_state(raw, &mut state) },
            YU_STORAGE_OK
        );
        assert_eq!(state.revision, 1);
        assert_eq!(state.dirty, 1);

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source_range(
                    raw,
                    1,
                    4,
                    7,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn clipboard_text_ffi_rejects_invalid_inputs_before_mutation() {
        let path = std::env::temp_dir().join(format!("yu-tsv-paste-{}.md", temp_id()));
        fs::write(&path, "").expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let mut result = YuStorageCommandResult::default();
        for (text, revision, format, status) in [
            ("a\tb", 0, 9, YU_STORAGE_INVALID_COMMAND),
            ("a\tb", 99, 1, YU_STORAGE_STALE_REVISION),
            ("\"unclosed", 0, 1, YU_STORAGE_INVALID_TABLE_PASTE),
        ] {
            assert_eq!(
                unsafe {
                    yu_storage_session_paste_text(
                        raw,
                        revision,
                        text.as_ptr(),
                        text.len(),
                        format,
                        &mut result,
                    )
                },
                status
            );
            assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), "");
            assert_eq!(unsafe { &*raw }.session.revision().get(), 0);
        }
        let text = "\" a\nb \"\tx";
        assert_eq!(
            unsafe {
                yu_storage_session_paste_text(raw, 0, text.as_ptr(), text.len(), 1, ptr::null_mut())
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(
            unsafe {
                yu_storage_session_paste_text(raw, 0, text.as_ptr(), text.len(), 1, &mut result)
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { &*raw }.session.snapshot().as_str(),
            "| &#32;a<br>b&#32; | x |\n| --- | --- |"
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn fragment_paste_ffi_validates_all_offsets_before_mutating() {
        let path = std::env::temp_dir().join(format!("yu-fragment-paste-{}.md", temp_id()));
        fs::write(&path, "original").expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let text = "羽a";
        let mut result = YuStorageCommandResult::default();
        for ends in [vec![1, 4], vec![3, 2, 4], vec![3], vec![5], vec![]] {
            assert_eq!(
                unsafe {
                    yu_storage_session_paste_fragments(
                        raw,
                        0,
                        text.as_ptr(),
                        text.len(),
                        ends.as_ptr(),
                        ends.len(),
                        0,
                        YU_STORAGE_FRAGMENT_TEXT,
                        &mut result,
                    )
                },
                YU_STORAGE_INVALID_SELECTION
            );
            assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), "original");
        }
        let ends = [3, 4];
        assert_eq!(
            unsafe {
                yu_storage_session_paste_fragments(
                    raw,
                    99,
                    text.as_ptr(),
                    text.len(),
                    ends.as_ptr(),
                    2,
                    0,
                    YU_STORAGE_FRAGMENT_TEXT,
                    &mut result,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_paste_fragments(
                    raw,
                    0,
                    text.as_ptr(),
                    text.len(),
                    ends.as_ptr(),
                    2,
                    0,
                    YU_STORAGE_FRAGMENT_TEXT,
                    ptr::null_mut(),
                )
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(
            unsafe {
                yu_storage_session_paste_fragments(
                    raw,
                    0,
                    text.as_ptr(),
                    text.len(),
                    ptr::null(),
                    2,
                    0,
                    YU_STORAGE_FRAGMENT_TEXT,
                    &mut result,
                )
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(
            unsafe {
                yu_storage_session_paste_fragments(
                    raw,
                    0,
                    text.as_ptr(),
                    text.len(),
                    ends.as_ptr(),
                    2,
                    0,
                    YU_STORAGE_FRAGMENT_TEXT,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { &*raw }.session.snapshot().as_str(),
            "羽\naoriginal"
        );
        let revision = unsafe { &*raw }.session.revision().get();
        let before = unsafe { &*raw }.session.snapshot().as_str().to_owned();
        for columns in [3, usize::MAX] {
            assert_eq!(
                unsafe {
                    yu_storage_session_paste_fragments(
                        raw,
                        revision,
                        text.as_ptr(),
                        text.len(),
                        ends.as_ptr(),
                        2,
                        columns,
                        YU_STORAGE_FRAGMENT_TEXT,
                        &mut result,
                    )
                },
                YU_STORAGE_INVALID_TABLE_PASTE
            );
            assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), before);
            assert_eq!(unsafe { &*raw }.session.revision().get(), revision);
        }
        assert_eq!(
            unsafe {
                yu_storage_session_paste_fragments(
                    raw,
                    revision,
                    text.as_ptr(),
                    text.len(),
                    ends.as_ptr(),
                    2,
                    2,
                    YU_STORAGE_FRAGMENT_TEXT,
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert!(
            unsafe { &*raw }
                .session
                .snapshot()
                .as_str()
                .contains("| 羽 | a |\n| --- | --- |")
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn image_property_ffi_validates_identity_and_copies_owned_utf8() {
        let path = std::env::temp_dir().join(format!("yu-image-properties-{}.md", temp_id()));
        let original = "🙂 ![羽](<a&amp;b.png>)\r\n";
        fs::write(&path, original).expect("image property FFI fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let mut info = YuStorageImageProperties::default();
        assert_eq!(
            unsafe { yu_storage_session_image_properties(raw, 0, 3, &mut info) },
            YU_STORAGE_OK
        );
        assert_eq!(info.start_utf16, 3);
        assert_eq!(
            unsafe { yu_storage_session_image_properties(raw, 0, 1, &mut info) },
            YU_STORAGE_INVALID_SELECTION
        );
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_image_property(
                    raw,
                    &info,
                    0,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        let mut destination = vec![0; required];
        assert_eq!(
            unsafe {
                yu_storage_session_copy_image_property(
                    raw,
                    &info,
                    0,
                    destination.as_mut_ptr(),
                    destination.len(),
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(destination, b"a&b.png");
        info.width = 200;
        let alt = "新图";
        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_update_image_properties(
                    raw,
                    &info,
                    destination.as_ptr(),
                    destination.len(),
                    alt.as_ptr(),
                    alt.len(),
                    ptr::null_mut(),
                )
            },
            YU_STORAGE_NULL_POINTER
        );
        assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), original);
        assert_eq!(
            unsafe {
                yu_storage_session_update_image_properties(
                    raw,
                    &info,
                    destination.as_ptr(),
                    destination.len(),
                    alt.as_ptr(),
                    alt.len(),
                    &mut result,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { &*raw }.session.snapshot().as_str(),
            "🙂 <img src=\"a&amp;b.png\" alt=\"新图\" width=\"200\">\r\n"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_update_image_properties(
                    raw,
                    &info,
                    destination.as_ptr(),
                    destination.len(),
                    alt.as_ptr(),
                    alt.len(),
                    &mut result,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_copy_image_property(
                    raw,
                    &info,
                    1,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(raw, YU_STORAGE_COMMAND_UNDO, 0, &mut result)
            },
            YU_STORAGE_OK
        );
        assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), original);
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("image property FFI fixture");
    }

    #[test]
    fn local_image_batch_ffi_is_atomic_and_one_undo_step() {
        let path = std::env::temp_dir().join(format!("yu-image-batch-{}.md", temp_id()));
        fs::write(&path, "原文\r\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let text = "assets/图 %.png甲other.png乙";
        let first_end = "assets/图 %.png".len();
        let second_start = first_end + "甲".len();
        let second_end = second_start + "other.png".len();
        let mut items = [
            YuStorageImageInput {
                path_start: 0,
                path_end: first_end,
                alternative_start: first_end,
                alternative_end: second_start,
            },
            YuStorageImageInput {
                path_start: second_start,
                path_end: second_end,
                alternative_start: second_end,
                alternative_end: text.len(),
            },
        ];
        let mut output = YuStorageCommandResult::default();
        let insert =
            |revision, items: &[YuStorageImageInput], output: *mut YuStorageCommandResult| unsafe {
                yu_storage_session_insert_local_images(
                    raw,
                    revision,
                    text.as_ptr(),
                    text.len(),
                    items.as_ptr(),
                    items.len(),
                    ptr::null(),
                    output,
                )
            };
        assert_eq!(insert(99, &items, &mut output), YU_STORAGE_STALE_REVISION);
        assert_eq!(insert(0, &items, ptr::null_mut()), YU_STORAGE_NULL_POINTER);
        items[1].path_end = second_start; // invalid later image must not publish the first
        assert_eq!(insert(0, &items, &mut output), YU_STORAGE_INVALID_PATH);
        assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), "原文\r\n");
        items[1].path_end = second_end;
        items[1].alternative_end = text.len() - 1; // splits the Chinese scalar
        assert_eq!(insert(0, &items, &mut output), YU_STORAGE_INVALID_SELECTION);
        assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), "原文\r\n");
        items[1].alternative_end = text.len();
        assert_eq!(insert(0, &items, &mut output), YU_STORAGE_OK);
        assert_eq!(
            unsafe { &*raw }.session.snapshot().as_str(),
            "![甲](assets/%E5%9B%BE%20%25.png) ![乙](other.png)原文\r\n"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(raw, YU_STORAGE_COMMAND_UNDO, 0, &mut output)
            },
            YU_STORAGE_OK
        );
        assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), "原文\r\n");
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("remove fixture");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_cell_paragraphs_keep_coretext_geometry_and_source_ranges() {
        let source = "<table><tr><td><p>第一段 office</p><div align='right'><p>second שלום</p><p><code>third</code></p></div></td><td>KEEP</td></tr></table>";
        let snapshot = yu_text::TextBuffer::new(source).snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("table");
        let decorations = yu_editor::DecorationCache::default()
            .decorate(&markdown, block, None)
            .expect("projection");
        let visual =
            yu_editor::VisualText::new(&snapshot, block.range(), decorations.set().clone())
                .expect("visual");
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        for width in [240.0, 500.0] {
            let view = yu_editor::BlockView::build_shaped(
                block.kind(),
                &visual,
                &decorations,
                LayoutConfig::new(width, 16.0),
                &shaper,
            )
            .expect("paragraph table");
            let table = view.table().expect("table");
            assert!(table.cell_layouts()[0].lines().len() >= 3);
            for cluster in view.clusters() {
                if cluster.width() < 0.1 {
                    continue;
                }
                let caret = view
                    .caret_for_source(cluster.source().start(), yu_editor::Bias::After)
                    .expect("caret");
                assert!((caret.point().y() - cluster.y()).abs() < 0.01);
                let hit = view
                    .hit_test(LayoutPoint::new(
                        cluster.x() + cluster.width() * 0.25,
                        cluster.y() + cluster.line_height() * 0.5,
                    ))
                    .expect("hit");
                assert_eq!(hit.content_source(), Some(cluster.source()));
            }
            for glyph in view.glyphs() {
                assert_eq!(
                    &source[glyph.source().start().get() as usize
                        ..glyph.source().end().get() as usize],
                    &visual.text()[glyph.visual().start().get() as usize
                        ..glyph.visual().end().get() as usize]
                );
            }
        }
        assert_eq!(snapshot.as_str(), source);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_cell_details_coretext_draws_headers_and_hits_empty_summaries() {
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        for summary in [
            "<summary>标题 office שלום</summary>",
            "<summary></summary>",
            "",
        ] {
            let source = format!(
                "<table><tr><td><details>{summary}<p>Hidden body</p></details></td><td>KEEP</td></tr></table>"
            );
            for (width, theme) in [240.0, 500.0].into_iter().flat_map(|width| {
                [
                    yu_core::ThemeId::YuLight,
                    yu_core::ThemeId::YuDark,
                    yu_core::ThemeId::Github,
                    yu_core::ThemeId::Night,
                ]
                .into_iter()
                .map(move |theme| (width, theme))
            }) {
                let mut doc = yu_editor::EditorDocument::new(&source);
                let config = LayoutConfig::new(width, 16.0).with_theme(theme);
                let view = doc
                    .block_layout_with_shaper(0, config, &shaper)
                    .expect("closed");
                let closed_height = view.table().expect("table").bounds().height();
                assert!(
                    view.glyphs()
                        .iter()
                        .any(|glyph| &source[glyph.source().start().get() as usize
                            ..glyph.source().end().get() as usize]
                            == "<details>"),
                    "disclosure arrow must be an actual glyph"
                );
                assert!(!view.visual().text().contains("Hidden body"));
                if !summary.is_empty() {
                    let at = ByteOffset::new(source.find("<summary>").expect("summary") as u64 + 9);
                    let caret = view
                        .caret_for_source(at, yu_editor::Bias::After)
                        .expect("summary caret");
                    assert_eq!(
                        view.hit_test(LayoutPoint::new(caret.point().x(), caret.point().y() + 1.0))
                            .expect("summary hit")
                            .source(),
                        at
                    );
                }
                let at = ByteOffset::new(source.find("<details>").expect("details") as u64);
                doc.toggle_html_details(at).expect("expand");
                let open = doc
                    .block_layout_with_shaper(0, config, &shaper)
                    .expect("open");
                assert!(
                    open.visual().text().contains("▾ ")
                        && open.visual().text().contains("Hidden body")
                );
                assert!(open.table().expect("open table").bounds().height() > closed_height);
                doc.undo().expect("undo");
                assert_eq!(doc.snapshot().as_str(), source);
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_cell_lists_use_coretext_markers_wrapping_and_empty_item_geometry() {
        let source = "<table><tr><td><ol start='9999'><li>outer 中文 office words words words<ul><li>nested שלום words words words</li><li></li></ul></li><li>second</li></ol></td><td>KEEP</td></tr></table>";
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        for theme in [
            yu_core::ThemeId::YuLight,
            yu_core::ThemeId::YuDark,
            yu_core::ThemeId::Github,
            yu_core::ThemeId::Night,
        ] {
            for width in [120.0, 240.0, 500.0] {
                let mut doc = yu_editor::EditorDocument::new(source);
                let config = LayoutConfig::new(width, 16.0).with_theme(theme);
                let view = doc
                    .block_layout_with_shaper(0, config, &shaper)
                    .expect("native lists");
                let table = view.table().expect("table");
                let marker_glyphs: Vec<_> = view
                    .glyphs()
                    .iter()
                    .filter(|glyph| glyph.visual().is_empty())
                    .collect();
                assert!(
                    marker_glyphs.len() >= 11,
                    "missing ordered/empty item markers"
                );
                for glyph in marker_glyphs {
                    assert_eq!(
                        &source[glyph.source().start().get() as usize
                            ..glyph.source().end().get() as usize],
                        "<li>"
                    );
                    assert!(glyph.size_scale() > 0.0 && glyph.size_scale() <= 1.0);
                    assert!(
                        table.cells()[0].bounds().contains(glyph.origin()),
                        "marker escaped cell: {:?}",
                        glyph.origin()
                    );
                    assert!(table.cell_layouts()[0].lines().iter().any(|line| {
                        (table.cells()[0].content_y() + line.y() + line.baseline()
                            - glyph.origin().y())
                        .abs()
                            < 0.1
                    }));
                }
                for label in ["outer", "nested", "second", "KEEP"] {
                    let at = ByteOffset::new(source.find(label).expect("label") as u64);
                    let caret = view
                        .caret_for_source(at, yu_editor::Bias::After)
                        .expect("caret");
                    let hit = view
                        .hit_test(LayoutPoint::new(caret.point().x(), caret.point().y() + 1.0))
                        .expect("hit");
                    assert_eq!(hit.source(), at, "{theme:?} {width} {label}");
                }
                let at = ByteOffset::new(source.find("<li></li>").expect("empty") as u64 + 4);
                let caret = view
                    .caret_for_source(at, yu_editor::Bias::After)
                    .expect("empty caret");
                assert_eq!(
                    view.hit_test(LayoutPoint::new(caret.point().x(), caret.point().y() + 1.0))
                        .expect("empty hit")
                        .source(),
                    at
                );
                doc.begin_composition(
                    yu_core::TextRange::empty(at),
                    "中文",
                    yu_core::Utf16Range::empty(Utf16Offset::new(2)),
                )
                .expect("preedit");
                let marked = doc
                    .block_layout_for_visual_state_with_shaper(0, config, &shaper)
                    .expect("marked list");
                assert!(
                    marked
                        .glyphs()
                        .iter()
                        .any(|glyph| !glyph.visual().is_empty() && glyph.source().is_empty())
                );
                assert_eq!(doc.snapshot().as_str(), source);
                assert!(doc.cancel_composition());
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn empty_html_list_items_have_coretext_markers_carets_and_editable_source() {
        let source = "<ol start='8'><li></li><li><!--keep--></li><li>尾</li></ol>";
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        let mut doc = yu_editor::EditorDocument::new(source);
        assert_eq!(doc.markdown().blocks().len(), 3);
        for (index, expected) in ["8.", "9.", "10."].into_iter().enumerate() {
            let view = doc
                .block_layout_with_shaper(index, LayoutConfig::new(400.0, 16.0), &shaper)
                .expect("empty list layout");
            assert_eq!(view.ornaments().marker().expect("marker").text(), expected);
            assert!(
                !view.glyphs().is_empty(),
                "empty item must still render its number"
            );
            assert!(view.lines()[0].height() > 0.0);
            let at =
                ByteOffset::new(source.match_indices("<li>").nth(index).expect("li").0 as u64 + 4);
            let caret = view
                .caret_for_source(at, yu_editor::Bias::After)
                .expect("empty caret");
            let hit = view
                .hit_test(LayoutPoint::new(caret.point().x(), caret.point().y() + 1.0))
                .expect("hit");
            assert_eq!(hit.source(), at, "click must insert inside the list item");
        }
        let at = ByteOffset::new(source.find("</li>").expect("empty item") as u64);
        doc.begin_composition(
            yu_core::TextRange::empty(at),
            "中文",
            yu_core::Utf16Range::empty(Utf16Offset::new(2)),
        )
        .expect("preedit");
        let view = doc
            .block_layout_for_visual_state_with_shaper(0, LayoutConfig::new(400.0, 16.0), &shaper)
            .expect("preedit layout");
        assert!(view.visual().text().contains("中文"));
        assert_eq!(doc.snapshot().as_str(), source);
        assert!(doc.cancel_composition());
        doc.begin_composition(
            yu_core::TextRange::empty(at),
            "中文",
            yu_core::Utf16Range::empty(Utf16Offset::new(2)),
        )
        .expect("preedit again");
        doc.commit_composition("中文").expect("commit");
        assert_eq!(
            doc.snapshot().as_str(),
            source.replacen("<li></li>", "<li>中文</li>", 1)
        );
        doc.undo().expect("undo");
        assert_eq!(doc.snapshot().as_str(), source);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_cell_heading_preedit_renders_at_both_boundaries_and_empty_heading() {
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        for body in ["", "body"] {
            let source = format!("<table><tr><td><h1>{body}</h1></td><td>KEEP</td></tr></table>");
            for at in [
                source.find("<h1>").expect("heading") + 4,
                source.find("</h1>").expect("end"),
            ] {
                let mut doc = yu_editor::EditorDocument::new(&source);
                doc.begin_composition(
                    yu_core::TextRange::empty(ByteOffset::new(at as u64)),
                    "中文",
                    yu_core::Utf16Range::empty(Utf16Offset::new(2)),
                )
                .expect("preedit");
                let view = doc
                    .block_layout_for_visual_state_with_shaper(
                        0,
                        LayoutConfig::new(500.0, 16.0),
                        &shaper,
                    )
                    .expect("marked heading");
                let start = view.visual().text().find("中文").expect("visual preedit") as u64;
                let glyphs: Vec<_> = view
                    .glyphs()
                    .iter()
                    .filter(|g| {
                        g.visual().start().get() >= start
                            && g.visual().end().get() <= start + "中文".len() as u64
                    })
                    .collect();
                assert!(
                    !glyphs.is_empty(),
                    "preedit has no visible glyphs: body={body:?} at={at}"
                );
                for glyph in glyphs {
                    assert!(
                        (glyph.size_scale() - yu_core::ThemeId::Github.spec().heading_sizes[0])
                            .abs()
                            < 0.001
                    );
                }
                assert_eq!(doc.snapshot().as_str(), source);
                assert!(doc.cancel_composition());
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_cell_headings_use_coretext_theme_styles_and_shared_geometry() {
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        for theme in [
            yu_core::ThemeId::YuLight,
            yu_core::ThemeId::YuDark,
            yu_core::ThemeId::Github,
            yu_core::ThemeId::Night,
        ] {
            for level in 1..=6 {
                let source = format!(
                    "<table><tr><td><h{level}>标题 <em>office</em> <code>code</code></h{level}><p>body שלום</p></td><td>KEEP</td></tr></table>"
                );
                let snapshot = yu_text::TextBuffer::new(&source).snapshot();
                let markdown = yu_markdown::parse(&snapshot);
                let block = markdown.blocks().get(0).expect("table");
                let decorations = yu_editor::DecorationCache::default()
                    .decorate(&markdown, block, None)
                    .expect("projection");
                let visual =
                    yu_editor::VisualText::new(&snapshot, block.range(), decorations.set().clone())
                        .expect("visual");
                for width in [240.0, 500.0] {
                    let view = yu_editor::BlockView::build_shaped(
                        block.kind(),
                        &visual,
                        &decorations,
                        LayoutConfig::new(width, 16.0).with_theme(theme),
                        &shaper,
                    )
                    .expect("heading table");
                    assert!(view.table().is_some());
                    for glyph in view.glyphs() {
                        let raw = &source[glyph.source().start().get() as usize
                            ..glyph.source().end().get() as usize];
                        assert_eq!(
                            raw,
                            &visual.text()[glyph.visual().start().get() as usize
                                ..glyph.visual().end().get() as usize]
                        );
                        let offset = glyph.source().start().get() as usize;
                        let in_heading = offset < source.find("</h").expect("heading end");
                        let in_code = offset > source.find("<code>").expect("code")
                            && offset < source.find("</code>").expect("code end");
                        let expected = if in_heading {
                            theme.spec().heading_sizes[level as usize - 1]
                                * if in_code {
                                    theme.inline_code_size_ratio(true)
                                } else {
                                    1.0
                                }
                        } else {
                            1.0
                        };
                        assert!(
                            (glyph.size_scale() - expected).abs() < 0.001,
                            "{theme:?} h{level} {raw}: {} != {expected}",
                            glyph.size_scale()
                        );
                        if in_heading && theme.spec().heading_bold[level as usize - 1] != 0 {
                            assert!(glyph.style().is_strong());
                        }
                    }
                    for cluster in view.clusters() {
                        if cluster.width() < 0.1 {
                            continue;
                        }
                        let caret = view
                            .caret_for_source(cluster.source().start(), yu_editor::Bias::After)
                            .expect("caret");
                        assert!((caret.point().y() - cluster.y()).abs() < 0.01);
                        let hit = view
                            .hit_test(LayoutPoint::new(
                                cluster.x() + cluster.width() * 0.25,
                                cluster.y() + cluster.line_height() * 0.5,
                            ))
                            .expect("hit");
                        assert_eq!(hit.content_source(), Some(cluster.source()));
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_table_justification_uses_each_cells_native_width_and_hit_geometry() {
        let text = "one two 中文 three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen";
        let source = format!(
            "<table align='justify'><tr><td>{text}</td><td align='right'>KEEP</td></tr></table>"
        );
        let snapshot = yu_text::TextBuffer::new(&source).snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("table");
        let decorations = yu_editor::DecorationCache::default()
            .decorate(&markdown, block, None)
            .expect("projection");
        let visual =
            yu_editor::VisualText::new(&snapshot, block.range(), decorations.set().clone())
                .expect("visual");
        assert_eq!(visual.text(), format!("{text}KEEP"));
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        let mut widths = Vec::new();
        for width in [220.0, 400.0] {
            let view = yu_editor::BlockView::build_shaped(
                block.kind(),
                &visual,
                &decorations,
                LayoutConfig::new(width, 16.0),
                &shaper,
            )
            .expect("native table");
            assert!(
                view.glyphs().iter().any(|glyph| {
                    source
                        [glyph.source().start().get() as usize..glyph.source().end().get() as usize]
                        .chars()
                        .count()
                        > 1
                }),
                "fixture must contain a native multi-grapheme glyph"
            );
            for glyph in view.glyphs() {
                let original = &source
                    [glyph.source().start().get() as usize..glyph.source().end().get() as usize];
                let projected = &visual.text()
                    [glyph.visual().start().get() as usize..glyph.visual().end().get() as usize];
                assert_eq!(original, projected, "ligature source coverage");
            }
            let table = view.table().expect("projected grid");
            assert_eq!(
                table.cells()[0].alignment(),
                yu_markdown::TableAlignment::Justify
            );
            assert_eq!(
                table.cells()[1].alignment(),
                yu_markdown::TableAlignment::Right
            );
            let cell = &table.cell_layouts()[0];
            let available = cell.config().max_width();
            widths.push(available);
            assert!(cell.lines().len() > 1);
            for line in &cell.lines()[..cell.lines().len() - 1] {
                assert!((line.width() - available).abs() < 0.01, "{width}: {line:?}");
            }
            assert!(cell.lines().last().expect("last").width() < available);
            for cluster in view.clusters() {
                if cluster.width() < 0.1 {
                    continue;
                }
                let caret = view
                    .caret_for_source(cluster.source().start(), yu_editor::Bias::After)
                    .expect("caret");
                assert!((caret.point().x() - cluster.x()).abs() < 0.01);
                let hit = view
                    .hit_test(LayoutPoint::new(
                        cluster.x() + cluster.width() * 0.25,
                        cluster.y() + cluster.line_height() * 0.5,
                    ))
                    .expect("hit");
                assert_eq!(hit.content_source(), Some(cluster.source()));
            }
        }
        assert!(widths[1] > widths[0]);
        assert_eq!(snapshot.as_str(), source);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn html_justification_reaches_coretext_and_shared_hit_geometry() {
        let text = "one two 中文 three four five six seven eight nine ten eleven twelve";
        let source = format!("<div align='justify'><p>{text}</p></div>");
        let snapshot = yu_text::TextBuffer::new(&source).snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("HTML paragraph");
        let decorations = yu_editor::DecorationCache::default()
            .decorate(&markdown, block, None)
            .expect("projection");
        let visual =
            yu_editor::VisualText::new(&snapshot, block.range(), decorations.set().clone())
                .expect("visual");
        assert_eq!(visual.text(), text);
        let shaper = CoreTextShaper::from_system_ui(
            yu_font::FontRequest::new("System UI", 16.0).expect("font"),
        )
        .expect("CoreText");
        let view = yu_editor::BlockView::build_shaped(
            block.kind(),
            &visual,
            &decorations,
            LayoutConfig::new(180.0, 16.0),
            &shaper,
        )
        .expect("native layout");
        assert!(view.lines().len() > 1);
        for line in &view.lines()[..view.lines().len() - 1] {
            assert!((line.width() - 180.0).abs() < 0.01, "{line:?}");
        }
        assert!(view.lines().last().expect("last").width() < 180.0);
        for cluster in view.clusters() {
            if cluster.width() < 0.1 {
                continue;
            }
            let caret = view
                .caret_for_source(cluster.source().start(), yu_editor::Bias::After)
                .expect("caret");
            assert!((caret.point().x() - cluster.x()).abs() < 0.01);
            let hit = view
                .hit_test(LayoutPoint::new(
                    cluster.x() + cluster.width() * 0.25,
                    cluster.y() + cluster.line_height() * 0.5,
                ))
                .expect("hit");
            assert_eq!(hit.content_source(), Some(cluster.source()));
        }
        assert_eq!(snapshot.as_str(), source);
    }

    #[test]
    fn html_cell_image_import_ffi_saves_elements_and_reopens_exactly() {
        for drop in [false, true] {
            let path = std::env::temp_dir().join(format!("yu-html-image-{}.md", temp_id()));
            let original = "\u{feff}<table><tr><td>中文</td><td>KEEP</td></tr></table>\r\n";
            fs::write(&path, original).expect("HTML image FFI fixture");
            let path_bytes = path.to_string_lossy().as_bytes().to_vec();
            let mut raw = ptr::null_mut();
            assert_eq!(
                unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
                YU_STORAGE_OK
            );
            // Storage removes only the BOM from its canonical editor source.
            let source = unsafe { &*raw }.session.snapshot().as_str().to_owned();
            let at = source[..source.find("中文").expect("HTML image FFI fixture")]
                .encode_utf16()
                .count() as u64;
            assert_eq!(
                unsafe {
                    yu_storage_session_set_selection_endpoints(
                        raw,
                        0,
                        at,
                        at,
                        YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    )
                },
                YU_STORAGE_OK
            );
            let text = "assets/图.png替代";
            let split = "assets/图.png".len();
            let item = YuStorageImageInput {
                path_start: 0,
                path_end: split,
                alternative_start: split,
                alternative_end: text.len(),
            };
            let target = YuStorageSelectionEndpoints {
                revision: 0,
                anchor_utf16: at,
                focus_utf16: at,
                affinity: YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            };
            let mut output = YuStorageCommandResult::default();
            assert_eq!(
                unsafe {
                    yu_storage_session_insert_local_images(
                        raw,
                        0,
                        text.as_ptr(),
                        text.len(),
                        &item,
                        1,
                        if drop { &target } else { ptr::null() },
                        &mut output,
                    )
                },
                YU_STORAGE_OK
            );
            let changed = unsafe { &*raw }.session.snapshot().as_str().to_owned();
            assert!(changed.contains("<img "), "{changed}");
            assert!(!changed.contains("!["));
            assert!(unsafe { &*raw }.session.document().is_dirty());
            let (mut revision, mut written, mut saved) = (0, 0, 0);
            assert_eq!(
                unsafe { yu_storage_session_save(raw, &mut revision, &mut written, &mut saved) },
                YU_STORAGE_OK
            );
            let bytes = fs::read(&path).expect("HTML image FFI fixture");
            assert_eq!(
                bytes,
                [b"\xef\xbb\xbf".as_slice(), changed.as_bytes()].concat()
            );
            assert_eq!(
                unsafe {
                    yu_storage_session_execute_command(raw, YU_STORAGE_COMMAND_UNDO, 0, &mut output)
                },
                YU_STORAGE_OK
            );
            assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), source);
            unsafe { yu_storage_session_destroy(raw) };
            assert_eq!(
                unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
                YU_STORAGE_OK
            );
            assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), changed);
            unsafe { yu_storage_session_destroy(raw) };
            fs::remove_file(path).expect("HTML image FFI fixture");
        }
    }

    #[test]
    fn image_drop_ffi_validates_target_before_any_selection_or_history_change() {
        let path = std::env::temp_dir().join(format!("yu-image-drop-{}.md", temp_id()));
        let original = "🙂 unchanged\r\n";
        fs::write(&path, original).expect("fixture");
        let bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(bytes.as_ptr(), bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    original.encode_utf16().count() as u64,
                    0,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let before = unsafe { &*raw }.session.selections().clone();
        let text = "a.pngimage";
        let mut item = YuStorageImageInput {
            path_start: 0,
            path_end: 5,
            alternative_start: 5,
            alternative_end: text.len(),
        };
        let target = YuStorageSelectionEndpoints {
            revision: 0,
            anchor_utf16: 3,
            focus_utf16: 3,
            affinity: YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
        };
        let insert = |target: &YuStorageSelectionEndpoints, item: &YuStorageImageInput| {
            let mut output = YuStorageCommandResult::default();
            unsafe {
                yu_storage_session_insert_local_images(
                    raw,
                    0,
                    text.as_ptr(),
                    text.len(),
                    item,
                    1,
                    target,
                    &mut output,
                )
            }
        };
        for (bad, expected) in [
            (
                YuStorageSelectionEndpoints {
                    revision: 9,
                    ..target
                },
                YU_STORAGE_STALE_REVISION,
            ),
            (
                YuStorageSelectionEndpoints {
                    anchor_utf16: 1,
                    focus_utf16: 1,
                    ..target
                },
                YU_STORAGE_INVALID_SELECTION,
            ),
            (
                YuStorageSelectionEndpoints {
                    focus_utf16: 4,
                    ..target
                },
                YU_STORAGE_INVALID_SELECTION,
            ),
        ] {
            assert_eq!(insert(&bad, &item), expected);
            assert_eq!(unsafe { &*raw }.session.selections(), &before);
            assert_eq!(unsafe { &*raw }.session.snapshot().as_str(), original);
        }
        item.path_end = 0;
        assert_eq!(insert(&target, &item), YU_STORAGE_INVALID_PATH);
        assert_eq!(unsafe { &*raw }.session.selections(), &before);
        item.path_end = 5;
        assert_eq!(insert(&target, &item), YU_STORAGE_OK);
        assert_eq!(
            unsafe { &*raw }.session.snapshot().as_str(),
            "🙂 ![image](a.png)unchanged\r\n"
        );
        assert_eq!(insert(&target, &item), YU_STORAGE_STALE_REVISION);
        let mut output = YuStorageCommandResult::default();
        assert_eq!(
            unsafe {
                yu_storage_session_execute_command(raw, YU_STORAGE_COMMAND_UNDO, 0, &mut output)
            },
            YU_STORAGE_OK
        );
        let session = unsafe { &*raw };
        assert_eq!(session.session.snapshot().as_str(), original);
        assert_eq!(
            session.session.selections().primary().anchor().get(),
            original.len() as u64
        );
        assert_eq!(
            session.session.selections().primary().focus(),
            ByteOffset::ZERO
        );
        assert_eq!(
            session.session.selections().primary().affinity(),
            CaretAffinity::Upstream
        );
        assert!(!session.session.command_available(&EditorCommand::Undo));
        assert_eq!(
            fs::read(&path).expect("unchanged disk"),
            original.as_bytes()
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unified_ffi_insert_text_is_revision_bound_and_returns_range_sync() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-insert-{id}.md"));
        fs::write(&path, "a").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        // 新文档的光标落在文首；这个用例断言的是「在末尾追加」，前提要自己建立。
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    1,
                    1,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let mut result = YuStorageCommandResult::default();
        let text = "日本語";
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, 99, text.as_ptr(), text.len(), &mut result)
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_insert_text(raw, 0, text.as_ptr(), text.len(), &mut result)
            },
            YU_STORAGE_OK
        );
        assert_eq!(result.revision, 1);
        assert_eq!(result.source_sync, YU_STORAGE_SOURCE_SYNC_RANGE);
        assert_eq!(result.source_start_utf16, 1);
        assert_eq!(result.source_old_end_utf16, 1);
        assert_eq!(result.source_new_end_utf16, 4);

        let mut required = 0;
        assert_eq!(
            unsafe { yu_storage_session_copy_source(raw, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_OK
        );
        let mut source = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_source(raw, source.as_mut_ptr(), source.len(), &mut written)
            },
            YU_STORAGE_OK
        );
        assert_eq!(String::from_utf8(source).expect("source"), "a日本語");
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unified_ffi_copy_selection_is_revision_bound_and_utf8_owned() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-copy-{id}.md"));
        fs::write(&path, "A🙂日本語Z").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    1,
                    6,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
                    YU_STORAGE_CLIPBOARD_TEXT,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        let mut selected = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
                    YU_STORAGE_CLIPBOARD_TEXT,
                    selected.as_mut_ptr(),
                    selected.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            String::from_utf8(selected).expect("selected UTF-8"),
            "🙂日本語"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    1,
                    YU_STORAGE_CLIPBOARD_TEXT,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unified_ffi_html_selection_is_source_revision_bound() {
        let id = temp_id();
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-html-{id}.md"));
        fs::write(&path, "**羽**").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection_endpoints(
                    raw,
                    0,
                    0,
                    5,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
                    YU_STORAGE_CLIPBOARD_HTML,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        let mut html = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
                    YU_STORAGE_CLIPBOARD_HTML,
                    html.as_mut_ptr(),
                    html.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        // **判据是「跨了 C ABI 之后还是那一段 HTML」，不是渲染器的逐字节输出。**
        // 这里以前钉的是 `"<p><strong>羽</strong></p>"` 整串，S7 第六刀把导出
        // 换成 comrak（它带一个收尾换行）之后就红了——而那次红说明不了任何
        // 关于 FFI 的事，渲染器的字节归 `yu-export` 自己的用例与 CommonMark
        // 棘轮管。这一层要压的是长度回报、缓冲区往返与 UTF-8 完整性。
        let html = String::from_utf8(html).expect("HTML UTF-8");
        assert_eq!(html.len(), written, "回报的长度与写出的字节数必须一致");
        assert!(
            html.contains("<strong>羽</strong>"),
            "选区那一段的语义要跨得过 C ABI：{html}"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    1,
                    YU_STORAGE_CLIPBOARD_HTML,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn html_import_ffi_is_stateless_two_call_and_rejects_unsafe_markup() {
        let html = "<h2>Yu</h2><p><strong>羽</strong></p>";
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_import_html_fragment(
                    html.as_ptr(),
                    html.len(),
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        let mut markdown = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_import_html_fragment(
                    html.as_ptr(),
                    html.len(),
                    markdown.as_mut_ptr(),
                    markdown.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            String::from_utf8(markdown).expect("Markdown UTF-8"),
            "## Yu\n\n**羽**"
        );

        let unsafe_html = "<img src=\"javascript:alert(1)\">";
        assert_eq!(
            unsafe {
                yu_storage_import_html_fragment(
                    unsafe_html.as_ptr(),
                    unsafe_html.len(),
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_HTML_IMPORT_REJECTED
        );
        let invalid_utf8 = [0xff_u8];
        assert_eq!(
            unsafe {
                yu_storage_import_html_fragment(
                    invalid_utf8.as_ptr(),
                    invalid_utf8.len(),
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_INVALID_UTF8
        );
    }
}
