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
use std::collections::{BTreeMap, HashSet};

use yu_assets::ImageKey;
use yu_core::{LineIndex, Revision, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    ACCESSIBILITY_SEMANTIC_FLAG_ORDERED, ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE,
    AccessibilitySemanticNode, AccessibilitySemanticSnapshot, AccessibilityTextError,
    AccessibilityTextSnapshot, BlockProjection, CaretAffinity, CaretScrollRequest, CommandResult,
    EditorCommand, EditorDocumentError, ImageSource, LayoutConfig, LayoutPoint, LayoutSnapshot,
    Projection, ProjectionBias, SelectionError, SourceSync, TableResizeCommit, TableResizeGesture,
    TableResizeGestureError, TableResizeHit, TableResizeTarget, ViewportConfig, ViewportRect,
    VisualOffset, VisualRunKind,
};
use yu_export::{ExportError, export_clipboard, import_html_fragment};
use yu_storage::{
    ClosePrompt, CloseRequest, CloseState, DiskState, DocumentEditorSession, ExternalFileState,
    SaveOutcome, StorageError, Utf8Bom,
};
use yu_text::{EditError, TextSnapshot};

#[cfg(target_os = "macos")]
use yu_scene::{EditorDecorationPrimitiveRole, Point, Primitive, Rect, Rgba8};
#[cfg(target_os = "macos")]
use yu_workspace::{EditorDecorationStyle, ViewportRenderConfig};

#[cfg(target_os = "macos")]
use yu_assets::{
    EmbeddedFailureKind, EmbeddedRenderRequest, EmbeddedRenderer, EmbeddedRequestResult,
    EmbeddedResourceCache, EmbeddedResourceKind, ImageCache, ImageFailureKind,
    ImageIntrinsicPublication, ImagePublication, ImageRequest, ImageRequestCandidate,
    ImageRequestPlan, ImageRequestPriority, ImageRequestResult,
};
#[cfg(target_os = "macos")]
use yu_embedded_math::MathRenderer;
#[cfg(target_os = "macos")]
use yu_font::FontRequest;
#[cfg(target_os = "macos")]
use yu_font::GlyphAtlasConfig;
#[cfg(target_os = "macos")]
use yu_font_macos::{CoreTextShaper, CoreTextViewportMetrics};
#[cfg(all(target_os = "macos", test))]
use yu_render_macos::MacosEmbeddedSvgRasterizer;
#[cfg(target_os = "macos")]
use yu_render_macos::{
    CoreTextViewportFrameBuilder, CoreTextViewportFrameError, MacosImageDecodeError,
    MacosImageDecodeWorker, MetalAtlas, MetalDevice, MetalFrameRenderer, MetalImageAtlas,
    MetalSurface, MetalSurfaceConfig, MetalUploader, MetalViewAttachmentOwned,
    MetalViewportHostSession,
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
pub const YU_STORAGE_EXPORT_ERROR: i32 = 17;
pub const YU_STORAGE_HTML_IMPORT_REJECTED: i32 = 18;
pub const YU_STORAGE_CORE_TEXT_UNAVAILABLE: i32 = 19;
pub const YU_STORAGE_INVALID_VIEWPORT_CONFIG: i32 = 20;
pub const YU_STORAGE_RENDER_HOST_UNAVAILABLE: i32 = 21;

pub const YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE: i32 = 22;

pub const YU_STORAGE_TABLE_ALIGNMENT_DEFAULT: u8 = 0;
pub const YU_STORAGE_TABLE_ALIGNMENT_LEFT: u8 = 1;
pub const YU_STORAGE_TABLE_ALIGNMENT_CENTER: u8 = 2;
pub const YU_STORAGE_TABLE_ALIGNMENT_RIGHT: u8 = 3;

pub const YU_STORAGE_TABLE_RESIZE_NONE: u8 = 0;
pub const YU_STORAGE_TABLE_RESIZE_COLUMN: u8 = 1;
pub const YU_STORAGE_TABLE_RESIZE_ROW: u8 = 2;

/// `yu_storage_session_macos_table_resize_at_point` 的动作。
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
pub const YU_STORAGE_EXTERNAL_CHANGED: u8 = YU_STORAGE_DISK_CHANGED;
pub const YU_STORAGE_EXTERNAL_MISSING: u8 = YU_STORAGE_DISK_MISSING;
pub const YU_STORAGE_ACCESSIBILITY_PARENT_NONE: u32 = u32::MAX;
pub const YU_STORAGE_ACCESSIBILITY_NO_RANGE: u64 = u64::MAX;
pub const YU_STORAGE_ACCESSIBILITY_NO_ACTION_BLOCK: u64 = u64::MAX;
pub const YU_STORAGE_ACCESSIBILITY_FLAG_ORDERED: u8 = ACCESSIBILITY_SEMANTIC_FLAG_ORDERED;
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
    /// 当前可见范围内还有未落定的图片或内嵌资源，需要再提交一次去收割 worker
    /// 的结果。这个判断此前在平台侧，要三次纯查询往返才能得出。
    pub resource_refresh_pending: u8,
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
    pub selection_decoration_count: u64,
    pub caret_decoration_count: u64,
    /// 见 [`YuStorageMacosRenderHostSnapshot::resource_refresh_pending`]。
    pub resource_refresh_pending: u8,
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
    renderer: Box<dyn EmbeddedRenderer>,
}

#[cfg(target_os = "macos")]
impl MacosEmbeddedResourceState {
    fn new() -> Self {
        Self {
            cache: EmbeddedResourceCache::new(),
            renderer: Box::new(MathRenderer::default()),
        }
    }

    fn request_result(
        &mut self,
        request: EmbeddedRenderRequest,
        revision: Revision,
    ) -> Result<EmbeddedRequestResult, i32> {
        let _ = self.cache.request(request.clone());
        while self
            .cache
            .render_pending(revision, self.renderer.as_ref())
            .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?
            .is_some()
        {}
        Ok(self.cache.request(request))
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

    fn sync(
        &mut self,
        plan: ImageRequestPlan,
        revision: yu_core::Revision,
        document_path: PathBuf,
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
struct MacosRenderHostState {
    builder: CoreTextViewportFrameBuilder,
    host: MetalViewportHostSession,
    size: f32,
    surface: Option<MacosPersistentSurfaceState>,
    image_resources: MacosImageResourceState,
    /// 上一次成功提交的帧标识，用于跳过完全等价的重复提交。
    ///
    /// 这个判断此前在 Swift：平台每帧都要先查 Revision 和 composition
    /// generation 才能组装出比较用的键，一次提交因此产生七八次 FFI 往返。
    /// 状态在 Rust，决策就该在 Rust。
    last_frame_key: Option<MacosFrameKey>,
}

/// 平台提供的一帧几何。
///
/// 这些值只有 AppKit 知道（view bounds、clip view 滚动位置、backing scale），
/// 因此必须由平台传入；其余判断全部留在 Rust。用位模式比较而不是浮点相等，
/// 避免 NaN 让「相同几何」永远判为不同而每帧重画。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq)]
struct MacosFrameGeometry {
    size_bits: u32,
    max_width_bits: u32,
    scroll_y_bits: u32,
    viewport_height_bits: u32,
    surface_width_bits: u64,
    surface_height_bits: u64,
    scale_bits: u64,
}

#[cfg(target_os = "macos")]
impl MacosFrameGeometry {
    fn from_request(request: &YuStorageFrameGeometry) -> Result<Self, i32> {
        let finite32 = |value: f32, positive: bool| {
            value.is_finite() && (if positive { value > 0.0 } else { value >= 0.0 })
        };
        let finite64 = |value: f64| value.is_finite() && value > 0.0;
        if !finite32(request.size, true)
            || !finite32(request.max_width, true)
            || !finite32(request.scroll_y, false)
            || !finite32(request.viewport_height, true)
            || !finite64(request.surface_width)
            || !finite64(request.surface_height)
            || !finite64(request.scale)
        {
            return Err(YU_STORAGE_EDITOR_ERROR);
        }
        Ok(Self {
            size_bits: request.size.to_bits(),
            max_width_bits: request.max_width.to_bits(),
            scroll_y_bits: request.scroll_y.to_bits(),
            viewport_height_bits: request.viewport_height.to_bits(),
            surface_width_bits: request.surface_width.to_bits(),
            surface_height_bits: request.surface_height.to_bits(),
            scale_bits: request.scale.to_bits(),
        })
    }
}

/// 表格 resize 的有效覆盖，作为帧身份的一部分。
///
/// 拖动分隔线既不推进 Revision 也不改变几何，但整张表的列宽都会变。少了这一项，
/// 一次拖动会被判为「与屏幕上的帧等价」而整段被跳过。
///
/// 与几何同理用位模式比较：`TableResizeCommit` 携带 f32，直接用 `PartialEq`
/// 会让任何 NaN 与自身不等，从而每帧重画。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq)]
struct MacosFrameTableResize {
    revision: u64,
    block_index: usize,
    target: TableResizeTarget,
    initial_position_bits: u32,
    final_position_bits: u32,
}

#[cfg(target_os = "macos")]
impl MacosFrameTableResize {
    fn capture(commit: TableResizeCommit) -> Self {
        Self {
            revision: commit.revision().get(),
            block_index: commit.block_index(),
            target: commit.target(),
            initial_position_bits: commit.initial_position().to_bits(),
            final_position_bits: commit.final_position().to_bits(),
        }
    }
}

/// 一帧的完整身份：Rust 拥有的可视状态 + 平台提供的几何。
///
/// 全部一起比较，任何一项变化都要重画：
///
/// - `revision`：源码改变。
/// - `composition_generation`：marked text 更新——它不推进 Revision。
/// - `selection`：光标与选区装饰改变——它同样不推进 Revision。
/// - `table_resize`：拖动中的列宽覆盖——既不推进 Revision 也不改变几何。
/// - `geometry`：字号、换行宽度、滚动、surface 尺寸与 backing scale。
///
/// 这个列表就是「帧内容取决于什么」的完整定义。新增一种不推进 Revision 的
/// 可视状态时必须同时加进来，否则它的变化会被静默跳过——本项目最危险的失败
/// 模式正是这种不报错的漏画。
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq)]
struct MacosFrameKey {
    revision: u64,
    composition_generation: u64,
    selection: yu_editor::EditorSelection,
    table_resize: Option<MacosFrameTableResize>,
    geometry: MacosFrameGeometry,
}

#[cfg(target_os = "macos")]
impl MacosFrameKey {
    /// 用当前会话状态与平台几何组装帧身份。
    ///
    /// 提交路径与 `frame_is_current` 共用这一个构造函数。两边各写一份是这个
    /// 判断最容易出错的地方：只要有一项不对称，就会出现「明明变了却判为等价」
    /// 或「明明没变却每帧重画」。
    fn capture(session: &YuStorageSession, geometry: MacosFrameGeometry) -> Self {
        let revision = session.session.revision();
        // 与 `macos_render_host_frame` 使用同一条过滤规则：只有匹配当前
        // Revision 的列覆盖会进入渲染配置，其余不影响画面，也就不该影响身份。
        let table_resize = session
            .table_resize_override
            .filter(|commit| {
                commit.revision() == revision
                    && matches!(commit.target(), TableResizeTarget::Column { .. })
            })
            .map(MacosFrameTableResize::capture);
        Self {
            revision: revision.get(),
            composition_generation: session.session.composition_generation(),
            selection: session.session.selection(),
            table_resize,
            geometry,
        }
    }
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
    session: DocumentEditorSession,
    table_resize_gesture: Option<TableResizeGesture>,
    table_resize_override: Option<TableResizeCommit>,
    #[cfg(target_os = "macos")]
    macos_render_host: Option<MacosRenderHostState>,
    #[cfg(target_os = "macos")]
    macos_embedded_resources: MacosEmbeddedResourceState,
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
        ExportError::InlineParse(_) => YU_STORAGE_EXPORT_ERROR,
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

fn projected_utf8(projection: &Projection) -> Result<String, i32> {
    let mut bytes = Vec::new();
    for run in projection.runs() {
        if matches!(run.kind(), VisualRunKind::HiddenSyntax) {
            continue;
        }
        let text = projection
            .text_for_run(*run)
            .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
        bytes.extend_from_slice(text.as_bytes());
    }
    String::from_utf8(bytes).map_err(|_| YU_STORAGE_EDITOR_ERROR)
}

fn visual_utf16_offset(projected: &str, visual: VisualOffset) -> Result<u64, i32> {
    let offset = usize::try_from(visual.get()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let prefix = projected
        .get(..offset)
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    u64::try_from(prefix.encode_utf16().count()).map_err(|_| YU_STORAGE_INVALID_SELECTION)
}

fn projection_bias_from_affinity(affinity: CaretAffinity) -> ProjectionBias {
    match affinity {
        CaretAffinity::Upstream => ProjectionBias::Before,
        CaretAffinity::Downstream => ProjectionBias::After,
    }
}

fn affinity_to_ffi(affinity: CaretAffinity) -> u8 {
    match affinity {
        CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
        CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
    }
}

fn table_source_utf16_range(snapshot: &TextSnapshot, source: TextRange) -> Result<(u64, u64), i32> {
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

fn table_resize_accessibility_metadata(
    snapshot: &TextSnapshot,
    revision: u64,
    block_index: u64,
    block_y: f32,
    divider_width: f32,
    table: &yu_editor::TableLayoutSnapshot,
) -> Result<Vec<YuStorageTableResizeAccessibilityDivider>, i32> {
    let column_count =
        u64::try_from(table.column_widths().len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    if column_count < 2 {
        return Ok(Vec::new());
    }
    let (table_source_start_utf16, table_source_end_utf16) =
        table_source_utf16_range(snapshot, table.source_range())?;
    let bounds = table.bounds();
    let width = divider_width.max(1.0);
    let y = block_y + bounds.y();
    if !block_y.is_finite()
        || !divider_width.is_finite()
        || divider_width <= 0.0
        || !y.is_finite()
        || !bounds.height().is_finite()
        || bounds.height() <= 0.0
    {
        return Err(YU_STORAGE_INVALID_SELECTION);
    }
    // 大约半个行高，并夹在 8–16 之间：一次调整要看得见，但不能一步跳过整列。
    let adjust_step = (table.row_height() * 0.5).clamp(8.0, 16.0);
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

fn block_caret_from_layout(
    session: &DocumentEditorSession,
    block_index: usize,
    source_utf16: u64,
    affinity: CaretAffinity,
    layout: &LayoutSnapshot,
    line_height: f32,
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
    let projected = projected_utf8(layout.projection())?;
    let visual_utf16 = visual_utf16_offset(&projected, caret.visual())?;
    let round_trip = layout
        .projection()
        .visual_to_source(caret.visual(), bias)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let round_trip_source_utf16 = snapshot
        .utf16_offset(round_trip)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let point = caret.point();
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
) -> Result<(), i32> {
    let published = session.session.viewport_config().layout();
    if (published.max_width() - max_width).abs() <= MACOS_VIEWPORT_CONFIG_TOLERANCE
        && (published.line_height() - metrics.line_height()).abs()
            <= MACOS_VIEWPORT_CONFIG_TOLERANCE
        && (published.default_advance() - metrics.default_advance()).abs()
            <= MACOS_VIEWPORT_CONFIG_TOLERANCE
    {
        return Ok(());
    }
    let layout = LayoutConfig::new(max_width, metrics.line_height())
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
fn core_text_system_ui_layout(
    size: f32,
    max_width: f32,
) -> Result<(CoreTextShaper, CoreTextViewportMetrics, LayoutConfig), i32> {
    if !size.is_finite() || size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    let request =
        FontRequest::new("System UI", size).map_err(|_| YU_STORAGE_CORE_TEXT_UNAVAILABLE)?;
    let shaper =
        CoreTextShaper::from_system_ui(request).map_err(|_| YU_STORAGE_CORE_TEXT_UNAVAILABLE)?;
    let metrics = shaper
        .viewport_metrics("M中🙂e\u{301}")
        .map_err(|_| YU_STORAGE_CORE_TEXT_UNAVAILABLE)?;
    let config = LayoutConfig::new(max_width, metrics.line_height())
        .with_default_advance(metrics.default_advance());
    Ok((shaper, metrics, config))
}

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

fn composition_projection(session: &mut DocumentEditorSession) -> Result<Projection, i32> {
    session.composition_projection().map_err(storage_status)
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
    projection: &Projection,
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
    projection: &Projection,
    projected: &str,
) -> Result<(u64, u64), i32> {
    let replacement = projection
        .composition_range()
        .ok_or(YU_STORAGE_NO_OVERLAY)?;
    let visual_start = projection
        .source_to_visual(replacement.start(), ProjectionBias::Before)
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
    let projected = projected_utf8(&projection)?;
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

fn composition_active_visual_caret(
    projection: &Projection,
    selection_start_utf16: u64,
    selection_end_utf16: u64,
) -> Result<(VisualOffset, ProjectionBias), i32> {
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
        .source_to_visual(replacement.start(), ProjectionBias::Before)
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
    Ok((VisualOffset::new(active), ProjectionBias::After))
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

/// Executes one vertical caret command against the current macOS shaped
/// block layouts. The command result still uses the ordinary source/selection
/// contract; CoreText is only a caller-owned layout provider for this query.
/// The host must have published matching viewport metrics first.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_move_vertical(
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
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
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
        let (shaper, metrics, config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(session, max_width, metrics) {
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
    let session = Box::new(YuStorageSession {
        session,
        table_resize_gesture: None,
        table_resize_override: None,
        #[cfg(target_os = "macos")]
        macos_render_host: None,
        #[cfg(target_os = "macos")]
        macos_embedded_resources: MacosEmbeddedResourceState::new(),
    });
    // SAFETY: the pointer is transferred to the native caller as an opaque
    // handle and is reclaimed only by `yu_storage_session_destroy`.
    unsafe { *output = Box::into_raw(session) };
    YU_STORAGE_OK
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
    let projection = match session.session.inline_projection_for_visual_state() {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let bias = projection_bias_from_affinity(affinity);
    let visual = match projection.source_to_visual(source, bias) {
        Ok(visual) => visual,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let projected = match projected_utf8(&projection) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
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
    let projection = match session.session.inline_projection_for_visual_state() {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let projected = match projected_utf8(&projection) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
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
        (ProjectionBias::Before, ProjectionBias::After)
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

/// Resolves a document-space point through the current CoreText-shaped block
/// layout. The endpoint is Revision-bound and uses the same published
/// viewport metrics as the native surface; it never asks the Swift/AppKit
/// mirror to approximate glyph positions. The returned `x`/`y` are snapped
/// document-space caret coordinates, while source/visual offsets are mapped
/// through the full lossless projection.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_projection_hit_test(
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
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
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
        let (shaper, metrics, layout_config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(session, max_width, metrics) {
            return status;
        }

        let query_y = point_y.max(0.0);
        let viewport = ViewportRect::new(query_y, metrics.line_height());
        let snapshot = {
            let document = session.session.document_mut().editor_mut();
            match document.visible_blocks_with_visual_state_and_shaper(viewport, &shaper) {
                Ok(snapshot) => snapshot,
                Err(error) => return status_from_editor_error(error),
            }
        };
        let mut selected = None;
        let mut best_distance = f32::INFINITY;
        for block in snapshot.blocks() {
            let top = block.y();
            let bottom = top + block.height();
            let distance = if query_y < top {
                top - query_y
            } else if query_y > bottom {
                query_y - bottom
            } else {
                0.0
            };
            if distance < best_distance {
                best_distance = distance;
                selected = Some(*block);
            }
        }
        let Some(block) = selected else {
            return YU_STORAGE_INVALID_SELECTION;
        };
        let layout = {
            let document = session.session.document_mut().editor_mut();
            match document.block_layout_for_visual_state_with_shaper(
                block.index(),
                layout_config,
                &shaper,
            ) {
                Ok(layout) => layout,
                Err(error) => return status_from_editor_error(error),
            }
        };
        if layout.lines().is_empty() {
            return YU_STORAGE_INVALID_SELECTION;
        }
        let local_y = (query_y - block.y()).max(0.0);
        let hit = match layout.hit_test(LayoutPoint::new(point_x, local_y)) {
            Ok(hit) => hit,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let projection = match session.session.inline_projection_for_visual_state() {
            Ok(projection) => projection,
            Err(error) => return storage_status(error),
        };
        let visual = match projection.source_to_visual(hit.source(), hit.bias()) {
            Ok(visual) => visual,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let projected = match projected_utf8(&projection) {
            Ok(projected) => projected,
            Err(status) => return status,
        };
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
        let point = hit.point();
        let document_y = block.y() + point.y();
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
                line: line_base.saturating_add(hit.line() as u64),
                x: point.x(),
                y: document_y,
                affinity: affinity_to_ffi(match hit.bias() {
                    ProjectionBias::Before => CaretAffinity::Upstream,
                    ProjectionBias::After => CaretAffinity::Downstream,
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

/// Resolves the active marked-text caret through an uncached CoreText-shaped
/// composition layout. Canonical source and Revision remain unchanged; the
/// expected generation guards the transient preedit. Caret geometry is local
/// to the owning parser block, while visual UTF-16 ranges use the full
/// transient projected stream.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_composition_shaped_caret(
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
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
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
        let (shaper, metrics, layout_config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(session, max_width, metrics) {
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
        let layout = match session
            .session
            .document_mut()
            .editor_mut()
            .block_layout_with_composition_and_shaper(block_index, layout_config, &shaper)
        {
            Ok(layout) => layout,
            Err(error) => return status_from_editor_error(error),
        };
        if layout.lines().is_empty() {
            return YU_STORAGE_INVALID_SELECTION;
        }
        let (block_visual, block_bias) = match composition_active_visual_caret(
            layout.projection(),
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
        let full_projected = match projected_utf8(&full_projection) {
            Ok(projected) => projected,
            Err(status) => return status,
        };
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
        let point = caret.point();
        let line_height = metrics.line_height();
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
    let (shaper, metrics, layout_config) = core_text_system_ui_layout(size, max_width)?;
    macos_publish_viewport_config(session, max_width, metrics)?;

    let query_y = point_y.max(0.0);
    let viewport = ViewportRect::new(query_y, metrics.line_height());
    let snapshot = {
        let document = session.session.document_mut().editor_mut();
        document
            .visible_blocks_with_shaper(viewport, &shaper)
            .map_err(status_from_editor_error)?
    };
    let mut selected = None;
    let mut best_distance = f32::INFINITY;
    for block in snapshot.blocks() {
        let top = block.y();
        let bottom = top + block.height();
        let distance = if query_y < top {
            top - query_y
        } else if query_y > bottom {
            query_y - bottom
        } else {
            0.0
        };
        if distance < best_distance {
            best_distance = distance;
            selected = Some(*block);
        }
    }
    let Some(block) = selected else {
        return Err(YU_STORAGE_INVALID_SELECTION);
    };
    let table_resize = match session.table_resize_override {
        Some(commit) if commit.revision().get() == expected_revision => {
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
    let layout = {
        let document = session.session.document_mut().editor_mut();
        let layout = if let Some(commit) =
            table_resize.filter(|commit| commit.block_index() == block.index())
        {
            document.block_layout_with_table_resize_and_shaper(
                block.index(),
                layout_config,
                &shaper,
                commit,
            )
        } else {
            document
                .block_layout_with_shaper(block.index(), layout_config, &shaper)
                .cloned()
        };
        layout.map_err(status_from_editor_error)?
    };
    let Some(table) = layout.table() else {
        return Err(YU_STORAGE_INVALID_SELECTION);
    };
    let local_y = query_y - block.y();
    let hit = table
        .resize_hit_test(LayoutPoint::new(point_x, local_y), tolerance)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    Ok((block.index(), hit))
}

/// Resolves a document-space point against task checkbox geometry from the
/// current persistent macOS render-host publication. This function never
/// reparses Markdown, rebuilds layout or mutates the editor. A native caller
/// may pass the returned block to the existing `ToggleTask` command only while
/// the returned Revision is still current.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_task_checkbox_hit_test(
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
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_table_resize_at_point(
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

/// 推进一次表格分隔线拖动。
///
/// update / finish / cancel 此前是三个独立 FFI，参数与前置条件完全一致，只在
/// 「对 gesture 做什么」上不同。它们是同一个指针手势的三个阶段，属于不变量 I3
/// 允许的「输入事件」这一类——一个带 action 的入口就够了。
///
/// - `UPDATE`：把 `pointer_position` 送进手势，返回本帧要用的临时几何。
/// - `FINISH`：结束手势，最终几何作为 session 级覆盖保留给后续帧。
///   不产生任何 Markdown transaction。
/// - `CANCEL`：结束手势并清除覆盖。没有手势时返回 OK——它同时用于清掉
///   finish 之后仍然保留的那份预览。
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
    session.table_resize_override = Some(commit);
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
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
    let (shaper, metrics, config) = core_text_system_ui_layout(size, max_width)?;
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
    let layout = session
        .session
        .block_layout_with_shaper(block_index, config, &shaper)
        .map_err(storage_status)?;
    block_caret_from_layout(
        &session.session,
        block_index,
        source_utf16,
        affinity,
        &layout,
        metrics.line_height(),
        1,
    )
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
pub unsafe extern "C" fn yu_storage_session_macos_source_caret(
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
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
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
pub unsafe extern "C" fn yu_storage_session_macos_table_resize_accessibility_dividers(
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
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
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
        let (shaper, metrics, layout_config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(session, max_width, metrics) {
            return status;
        }

        let viewport = ViewportRect::new(scroll_y, viewport_height);
        let viewport_snapshot = {
            let document = session.session.document_mut().editor_mut();
            if document.composition().is_some() {
                return YU_STORAGE_OK;
            }
            match document.visible_blocks_with_shaper(viewport, &shaper) {
                Ok(snapshot) => snapshot,
                Err(error) => return status_from_editor_error(error),
            }
        };
        let source = session.session.snapshot();
        let divider_width = (metrics.default_advance() * 0.25).max(1.0);
        // Accessibility actions keep a session-only table preview alive after
        // each increment/decrement. Reuse that override here so the next
        // descriptor enumeration exposes the effective divider position
        // instead of resetting VoiceOver to the canonical layout.
        let table_resize = match session.table_resize_override {
            Some(commit) if commit.revision() == viewport_snapshot.revision() => {
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
        let mut encoded = Vec::new();
        for block in viewport_snapshot.blocks() {
            let layout = {
                let document = session.session.document_mut().editor_mut();
                let layout = if let Some(commit) =
                    table_resize.filter(|commit| commit.block_index() == block.index())
                {
                    document.block_layout_with_table_resize_and_shaper(
                        block.index(),
                        layout_config,
                        &shaper,
                        commit,
                    )
                } else {
                    document
                        .block_layout_with_shaper(block.index(), layout_config, &shaper)
                        .cloned()
                };
                match layout {
                    Ok(layout) => layout,
                    Err(error) => return status_from_editor_error(error),
                }
            };
            let Some(table) = layout.table() else {
                continue;
            };
            let block_index = match u64::try_from(block.index()) {
                Ok(index) => index,
                Err(_) => return YU_STORAGE_INVALID_SELECTION,
            };
            let metadata = match table_resize_accessibility_metadata(
                &source,
                viewport_snapshot.revision().get(),
                block_index,
                block.y(),
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

#[cfg(target_os = "macos")]
fn image_destination(
    source: &TextSnapshot,
    image: ImageSource,
    definitions: &yu_markdown::ReferenceDefinitionIndex,
) -> Option<TextRange> {
    image.destination().or_else(|| {
        image
            .reference()
            .and_then(|reference| definitions.lookup(source, reference))
            .map(|definition| definition.destination())
    })
}

#[cfg(target_os = "macos")]
fn image_resource_key(
    source: &TextSnapshot,
    image: ImageSource,
    definitions: &yu_markdown::ReferenceDefinitionIndex,
) -> Option<ImageKey> {
    let destination = image_destination(source, image, definitions)?;
    let start = usize::try_from(destination.start().get()).ok()?;
    let end = usize::try_from(destination.end().get()).ok()?;
    let destination = source.as_str().get(start..end)?;
    ImageKey::new(destination.to_owned()).ok()
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
    if resources
        .cache
        .failure(key)
        .is_some_and(|failure| failure.revision().get() == revision)
    {
        return YU_STORAGE_IMAGE_RESOURCE_FAILED;
    }
    if resources.in_flight.contains(key) || resources.intrinsics.contains_key(&fingerprint) {
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
        let projection = session
            .session
            .block_projection(block_index)
            .map_err(storage_status)?;
        for image in projection.visual().images().iter().copied() {
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
fn embedded_resource_kind(source: &TextSnapshot, info: TextRange) -> Option<u8> {
    let start = usize::try_from(info.start().get()).ok()?;
    let end = usize::try_from(info.end().get()).ok()?;
    let language = source.as_str().get(start..end)?.split_whitespace().next()?;
    if language.eq_ignore_ascii_case("mermaid") {
        Some(YU_STORAGE_EMBEDDED_MERMAID)
    } else if language.eq_ignore_ascii_case("math")
        || language.eq_ignore_ascii_case("latex")
        || language.eq_ignore_ascii_case("tex")
        || language.eq_ignore_ascii_case("katex")
    {
        Some(YU_STORAGE_EMBEDDED_MATH)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn embedded_resource_kind_from_ffi(kind: u8) -> Option<EmbeddedResourceKind> {
    match kind {
        YU_STORAGE_EMBEDDED_MATH => Some(EmbeddedResourceKind::Math),
        YU_STORAGE_EMBEDDED_MERMAID => Some(EmbeddedResourceKind::Mermaid),
        _ => None,
    }
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
fn embedded_resource_content(source: &TextSnapshot, content: TextRange) -> Result<String, i32> {
    let start = usize::try_from(content.start().get()).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let end = usize::try_from(content.end().get()).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    source
        .as_str()
        .get(start..end)
        .map(str::to_owned)
        .ok_or(YU_STORAGE_EDITOR_ERROR)
}

#[cfg(target_os = "macos")]
fn embedded_resource_fingerprint(source: &TextSnapshot, source_range: TextRange, kind: u8) -> u64 {
    let start = usize::try_from(source_range.start().get()).ok();
    let end = usize::try_from(source_range.end().get()).ok();
    let bytes = start
        .zip(end)
        .and_then(|(start, end)| source.as_str().as_bytes().get(start..end));
    let mut hash = 1_469_598_103_934_665_603_u64 ^ u64::from(kind);
    if let Some(bytes) = bytes {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(1_099_511_628_211_u64);
        }
    }
    hash
}

#[cfg(target_os = "macos")]
fn macos_render_host_error_status(error: &CoreTextViewportFrameError) -> i32 {
    match error {
        CoreTextViewportFrameError::InvalidConfig(_) => YU_STORAGE_INVALID_VIEWPORT_CONFIG,
        CoreTextViewportFrameError::Font(_) | CoreTextViewportFrameError::Raster(_) => {
            YU_STORAGE_CORE_TEXT_UNAVAILABLE
        }
        CoreTextViewportFrameError::Document(error) => status_from_editor_error(error.clone()),
        CoreTextViewportFrameError::Atlas(_)
        | CoreTextViewportFrameError::Publish(_)
        | CoreTextViewportFrameError::Host(_) => YU_STORAGE_RENDER_HOST_UNAVAILABLE,
    }
}

#[cfg(target_os = "macos")]
fn macos_render_host_config(
    viewport: ViewportRect,
    size: f32,
    max_width: f32,
    viewport_height: f32,
    raster_scale: f32,
) -> Result<ViewportRenderConfig, i32> {
    let scene_height = viewport_height.max(1.0);
    let scene_viewport = Rect::new(0.0, viewport.scroll_y(), max_width, scene_height)
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    Ok(
        ViewportRenderConfig::new(viewport, size, scene_viewport, Rgba8::black())
            .with_raster_scale(raster_scale)
            // Rust surface 是唯一渲染路径，背景必须由这一帧自己画出来
            // （不变量 I5）。暗色模式需要平台把实际的 textBackgroundColor
            // 传进来，目前固定为白底。
            .with_background(Rgba8::white())
            .with_editor_decorations(EditorDecorationStyle::new(
                Rgba8::new(0, 122, 255, 97),
                Rgba8::black(),
                Rgba8::new(0, 122, 255, 255),
                1.0,
            )),
    )
}

#[cfg(target_os = "macos")]
fn macos_editor_decoration_counts(scene: &yu_scene::Scene) -> Result<(u64, u64), i32> {
    let mut selection = 0_u64;
    let mut caret = 0_u64;
    for primitive in scene.primitives() {
        let Primitive::EditorDecoration(decoration) = primitive else {
            continue;
        };
        match decoration.role() {
            EditorDecorationPrimitiveRole::Selection => {
                selection = selection
                    .checked_add(1)
                    .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
            }
            EditorDecorationPrimitiveRole::Caret
            | EditorDecorationPrimitiveRole::CompositionCaret => {
                caret = caret
                    .checked_add(1)
                    .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
            }
        }
    }
    Ok((selection, caret))
}

#[cfg(target_os = "macos")]
fn macos_render_host_snapshot(
    state: &MacosRenderHostState,
    composition_generation: u64,
) -> Result<YuStorageMacosRenderHostSnapshot, i32> {
    let publication = state
        .builder
        .last_publication()
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
    let (selection_decoration_count, caret_decoration_count) =
        macos_editor_decoration_counts(frame.scene().scene())?;
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
        // 调用方在离开 host 借用之后填入；这里没有 session 可查。
        resource_refresh_pending: 0,
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
    surface_generation: u64,
    /// backing scale：字形按它取样，后端再除回逻辑坐标。
    raster_scale: f32,
}

#[cfg(target_os = "macos")]
fn macos_render_host_frame(
    session: &mut YuStorageSession,
    request: MacosFrameRequest,
) -> Result<YuStorageMacosRenderHostSnapshot, i32> {
    let MacosFrameRequest {
        expected_revision,
        size,
        max_width,
        scroll_y,
        viewport_height,
        surface_generation,
        raster_scale,
    } = request;
    validate_revision(&session.session, expected_revision)?;
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
    let rebuild = session
        .macos_render_host
        .as_ref()
        .is_none_or(|state| (state.size - size).abs() > 0.001);
    let (metrics, shaper) = if rebuild {
        let (shaper, metrics, _layout_config) = core_text_system_ui_layout(size, max_width)?;
        (metrics, Some(shaper))
    } else {
        let state = session
            .macos_render_host
            .as_ref()
            .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
        let metrics = state
            .builder
            .shaper()
            .viewport_metrics("M中🙂e\u{301}")
            .map_err(|_| YU_STORAGE_CORE_TEXT_UNAVAILABLE)?;
        (metrics, None)
    };
    macos_publish_viewport_config(session, max_width, metrics)?;

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
    let viewport = ViewportRect::new(scroll_y, viewport_height);
    let config =
        macos_render_host_config(viewport, size, max_width, viewport_height, raster_scale)?;
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
            .and_then(|state| state.builder.last_publication())
            .map_or(0, |publication| publication.serial());
        let builder = CoreTextViewportFrameBuilder::with_shaper_and_initial_serial(
            shaper.ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?,
            config,
            GlyphAtlasConfig::default(),
            initial_serial,
        )
        .map_err(|error| macos_render_host_error_status(&error))?;
        let previous = session.macos_render_host.take();
        let (surface, image_resources) = match previous {
            Some(state) => (state.surface, state.image_resources),
            None => (None, MacosImageResourceState::new()?),
        };
        session.macos_render_host = Some(MacosRenderHostState {
            builder,
            host: MetalViewportHostSession::new(revision, surface_generation),
            size,
            surface,
            image_resources,
            // 重建 host 后没有可复用的帧，下一次提交必须真正执行。
            last_frame_key: None,
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
    let document_path = session.session.path().to_path_buf();
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
    let image_requests = macos_image_requests(session, &viewport_blocks)?;
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
    state
        .image_resources
        .sync(image_requests, revision, document_path)?;
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
    let publication = {
        let document = session.session.document_mut().editor_mut();
        state
            .builder
            .publish_with_images_and_intrinsics(document, &image_publications, &image_intrinsics)
            .map_err(|error| macos_render_host_error_status(&error))?
    };
    state
        .host
        .accept_publication(publication)
        .map_err(|_| YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let mut snapshot = macos_render_host_snapshot(state, session.session.composition_generation())?;
    // 在离开 host 的可变借用之后再问：这一帧的可见资源是否已经全部落定。
    let visible_blocks = viewport_blocks
        .iter()
        .map(|&(index, _)| index)
        .collect::<Vec<_>>();
    snapshot.resource_refresh_pending = u8::from(macos_frame_needs_resource_refresh(
        session,
        &visible_blocks,
    )?);
    Ok(snapshot)
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
fn macos_frame_needs_resource_refresh(
    session: &mut YuStorageSession,
    visible_blocks: &[usize],
) -> Result<bool, i32> {
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
    for &block_index in visible_blocks {
        let projection = session
            .session
            .block_projection(block_index)
            .map_err(storage_status)?;
        for image in projection.visual().images().iter().copied() {
            let key = image_resource_key(&source, image, &definitions);
            let fingerprint = key.as_ref().map_or(0, ImageKey::fingerprint);
            let status = macos_image_resource_status(
                session.macos_render_host.as_ref(),
                key.as_ref(),
                revision.get(),
            );
            if macos_resource_status_needs_refresh(status, fingerprint) {
                pending = true;
            }
        }
        let BlockProjection::FencedCode(code) = projection else {
            continue;
        };
        let Some(kind) = embedded_resource_kind(&source, code.info_string()) else {
            continue;
        };
        let embedded_kind = embedded_resource_kind_from_ffi(kind).ok_or(YU_STORAGE_EDITOR_ERROR)?;
        let content = embedded_resource_content(&source, code.content())?;
        // 空的 fenced body 仍然是合法的源码资源，用一个 trim 中性的哨兵保持它
        // 走同一条缓存路径，与 `macos_visual_embedded_resources` 一致。
        let content = if content.is_empty() {
            "\n".to_owned()
        } else {
            content
        };
        let request =
            EmbeddedRenderRequest::new(revision, code.source_range(), embedded_kind, content)
                .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
        let status = session
            .macos_embedded_resources
            .status_for(request, revision)?;
        let fingerprint = embedded_resource_fingerprint(&source, code.source_range(), kind);
        if macos_resource_status_needs_refresh(status, fingerprint) {
            pending = true;
        }
    }
    Ok(pending)
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
fn macos_render_host_surface_prepare(
    session: &mut YuStorageSession,
    view: std::ptr::NonNull<c_void>,
    surface_width: f64,
    surface_height: f64,
    scale: f64,
) -> Result<u64, i32> {
    let config = MetalSurfaceConfig::new(surface_width, surface_height, scale)
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
    let renderer = match MetalFrameRenderer::new(device.clone()) {
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
        );
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
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
                surface_generation,
                raster_scale: 1.0,
            },
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
            view,
        );
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        use std::ptr::NonNull;

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
        let host_snapshot = match macos_render_host_frame(
            session,
            MacosFrameRequest {
                expected_revision,
                size,
                max_width,
                scroll_y,
                viewport_height,
                surface_generation,
                raster_scale: scale as f32,
            },
        ) {
            Ok(snapshot) => snapshot,
            Err(status) => return status,
        };
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
        let submission = match state.host.submit_with_images(
            &mut surface_state.renderer,
            &surface_state.surface,
            &mut surface_state.uploader,
            &mut surface_state.atlas,
            &mut surface_state.image_atlas,
        ) {
            Ok(submission) => submission,
            Err(_) => return YU_STORAGE_RENDER_HOST_UNAVAILABLE,
        };
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
                selection_decoration_count: host_snapshot.selection_decoration_count,
                caret_decoration_count: host_snapshot.caret_decoration_count,
                resource_refresh_pending: host_snapshot.resource_refresh_pending,
                content_height: host_snapshot.content_height,
            };
        }
        // 记录这一帧的身份，供 `frame_is_current` 判断后续提交是否等价。
        if let Ok(geometry) = MacosFrameGeometry::from_request(&YuStorageFrameGeometry {
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_width,
            surface_height,
            scale,
        }) {
            let key = MacosFrameKey::capture(session, geometry);
            if let Some(state) = session.macos_render_host.as_mut() {
                state.last_frame_key = Some(key);
            }
        }
        YU_STORAGE_OK
    }
}

/// 判断按给定几何提交的下一帧是否与已在屏幕上的帧完全等价。
///
/// 等价的完整定义见 [`MacosFrameKey`]：Revision、composition generation、
/// selection、表格 resize 覆盖与几何全部不变才算等价。其中前四项里有三项
/// 不推进 Revision，只比 Revision 会把光标移动、preedit 更新与列宽拖动
/// 全部静默跳过。
///
/// 这个判断此前在平台侧：Swift 每帧先查 Revision、再查 composition
/// generation，才能组装出比较用的键，一次提交因此产生多次纯查询往返。状态在
/// Rust，决策也该在 Rust（不变量 I3）。
///
/// # Safety
/// `session`、`geometry` 与 `out_current` 必须是调用方拥有的有效指针。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_frame_is_current(
    session: *mut YuStorageSession,
    geometry: *const YuStorageFrameGeometry,
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
        let _ = (session, geometry);
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        let geometry = match MacosFrameGeometry::from_request(geometry) {
            Ok(geometry) => geometry,
            Err(status) => return status,
        };
        let key = MacosFrameKey::capture(session, geometry);
        let current = session.macos_render_host.as_ref().is_some_and(|state| {
            // 没有 surface 时不能声称当前帧有效：内容还没有真正上屏。
            state.surface.is_some() && state.last_frame_key == Some(key)
        });
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
    #[cfg(target_os = "macos")]
    if let Some(state) = session.macos_render_host.as_mut() {
        state.surface.take();
        // 记录的帧已经随 surface 一起消失，不能留下来让下一次绑定误判等价。
        state.last_frame_key = None;
    }
    YU_STORAGE_OK
}

/// Resolves the current focus caret through the macOS CoreText-shaped
/// viewport policy. `caret_y` is document-space, while `current_scroll_y` and
/// `target_scroll_y` are absolute document scroll offsets. The native host
/// must apply the target only when the returned Revision still matches.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_shaped_caret_scroll_request(
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
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        if !size.is_finite() || size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
            return YU_STORAGE_EDITOR_ERROR;
        }
        let (shaper, metrics, _layout_config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if let Err(status) = macos_publish_viewport_config(session, max_width, metrics) {
            return status;
        }
        // 露出光标时在视口边缘留出的余量。此前由平台传入，而平台算的正是
        // `max(line_height, 4.0)`——一个 Rust 自己就知道的值。为了拿到它，
        // 平台必须先查一次字体度量，于是每次光标移动多一次纯往返。
        let margin = metrics.line_height().max(4.0);
        let request = match session.session.caret_scroll_request_with_shaper(
            ViewportRect::new(scroll_y, viewport_height),
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

/// Copies the current Rust-owned selection as UTF-8. The expected revision
/// makes the operation safe for a native clipboard callback that was queued
/// before a source edit.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_selection(
    session: *const YuStorageSession,
    expected_revision: u64,
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
    let range = session.session.selection().ordered_range();
    let snapshot = session.session.snapshot();
    write_snapshot_range(&snapshot, range, output, capacity, written)
}

/// Copies the current Rust-owned selection as a semantic HTML fragment. The
/// expected revision protects a queued native clipboard callback from reading
/// a newer selection/source revision.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_selection_html(
    session: *const YuStorageSession,
    expected_revision: u64,
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
    let range = session.session.selection().ordered_range();
    let snapshot = session.session.snapshot();
    let payload = match export_clipboard(&snapshot, session.session.revision(), range) {
        Ok(payload) => payload,
        Err(error) => return status_from_export_error(error),
    };
    write_bytes(payload.html().as_bytes(), output, capacity, written)
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

#[cfg(test)]
mod tests {
    use yu_core::ByteOffset;

    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn status_and_state_contracts_are_stable() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-{id}.md"));
        fs::write(&path, "羽 日本語 🙂\n").expect("fixture");
        let editor_session = DocumentEditorSession::open(&path).expect("open");
        let session = YuStorageSession {
            session: editor_session,
            table_resize_gesture: None,
            table_resize_override: None,
            #[cfg(target_os = "macos")]
            macos_render_host: None,
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
    fn macos_embedded_state_defaults_to_math_and_keeps_mermaid_unsupported() {
        let mut state = MacosEmbeddedResourceState::new();
        let request = EmbeddedRenderRequest::new(
            Revision::INITIAL,
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(3)).expect("range"),
            EmbeddedResourceKind::Math,
            "x^2",
        )
        .expect("request");
        assert_eq!(
            state
                .status_for(request.clone(), Revision::INITIAL)
                .expect("status"),
            YU_STORAGE_EMBEDDED_RESOURCE_READY
        );
        let publication = state
            .publication_for(request, Revision::INITIAL)
            .expect("publication")
            .expect("ready math publication");
        let yu_assets::EmbeddedRenderPayload::Svg { dimensions, markup } = publication.payload()
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
            "flowchart TD",
        )
        .expect("request");
        assert_eq!(
            state
                .status_for(mermaid, Revision::INITIAL)
                .expect("status"),
            YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                .inline_projection_for_visual_state()
                .expect("projection");
            assert_eq!(
                projected_utf8(&projection).expect("projected"),
                "羽 链接 🙂\n"
            );
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                .inline_projection_for_visual_state()
                .expect("projection");
            assert_eq!(projected_utf8(&projection).expect("projected"), source);
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
                .inline_projection_for_visual_state()
                .expect("projection");
            assert_eq!(
                projected_utf8(&projection).expect("projected"),
                "before strong after"
            );
        }

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_shaped_vertical_command_preserves_revision_and_selection_contract() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                yu_storage_session_macos_move_vertical(
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
                yu_storage_session_macos_move_vertical(
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
                yu_storage_session_macos_move_vertical(
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                yu_storage_session_macos_projection_hit_test(
                    raw, 0, 0.0, 0.0, 14.0, 500.0, &mut hit,
                )
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
                yu_storage_session_macos_projection_hit_test(
                    raw, 0, 0.0, 0.0, 14.0, 500.0, &mut hit,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(hit.revision, 0);
        assert_eq!(hit.visual_utf16, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_task_checkbox_hit_uses_current_published_frame_and_canonical_command() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                    raw, 0, 14.0, 500.0, 0.0, 240.0, 0, &mut frame,
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
                            Primitive::TaskCheckbox(task)
                                if task.role() == yu_scene::TaskCheckboxPrimitiveRole::Border =>
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
                yu_storage_session_macos_task_checkbox_hit_test(raw, 0, point_x, point_y, &mut hit)
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
                yu_storage_session_macos_task_checkbox_hit_test(
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
                yu_storage_session_macos_task_checkbox_hit_test(raw, 0, point_x, point_y, &mut hit)
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
                yu_storage_session_macos_task_checkbox_hit_test(raw, 0, point_x, point_y, &mut hit)
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
    fn macos_render_host_config_tracks_document_scroll_origin() {
        let viewport = ViewportRect::new(137.5, 240.0);
        let config = macos_render_host_config(viewport, 14.0, 500.0, 240.0, 2.0)
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
            unsafe { yu_storage_session_macos_frame_is_current(raw, &geometry, &mut current) },
            YU_STORAGE_OK
        );
        assert_eq!(current, 0, "尚未提交任何帧时不得判为当前");

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
                unsafe { yu_storage_session_macos_frame_is_current(raw, &invalid, &mut out) },
                YU_STORAGE_EDITOR_ERROR
            );
            assert_eq!(out, 0);
        }

        // 空指针必须返回明确状态，不得写入半成品输出。
        let mut out = 1_u8;
        assert_eq!(
            unsafe { yu_storage_session_macos_frame_is_current(raw, std::ptr::null(), &mut out) },
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
    /// 反向验证：把 `selection` 从 `MacosFrameKey` 去掉，第二段断言失败；
    /// 把 `table_resize` 去掉，第三段断言失败。
    /// PROBE 不得改变状态，未知 action 不得被当成 BEGIN 执行。
    ///
    /// 探测与开始拖动合成一个入口之后，「hover 只读」这条约束从两份实现各自
    /// 保证变成了一处 action 分支。分错会在鼠标划过表格时静默开出一个手势，
    /// 后续的 update 就会真的改列宽——没有报错。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_table_resize_probe_does_not_open_a_gesture() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-resize-probe-{id}.md"));
        fs::write(&path, "| A | B |\n| --- | :---: |\n| 1 | 2 |\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        // 先探出一个真实存在的分隔线位置。
        let mut probe = YuStorageTableResizeHit::default();
        let mut divider = None;
        for step in 0_u16..400 {
            let x = f32::from(step) * 0.5;
            let status = unsafe {
                yu_storage_session_macos_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_PROBE,
                    14.0,
                    500.0,
                    x,
                    1.0,
                    0.4,
                    0.0,
                    &mut probe,
                )
            };
            if status == YU_STORAGE_OK {
                divider = Some(x);
                break;
            }
        }
        let divider = divider.expect("a column divider must be reachable");
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
                yu_storage_session_macos_table_resize_at_point(
                    raw,
                    0,
                    9,
                    14.0,
                    500.0,
                    divider,
                    1.0,
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
                yu_storage_session_macos_table_resize_at_point(
                    raw,
                    0,
                    YU_STORAGE_TABLE_RESIZE_BEGIN,
                    14.0,
                    500.0,
                    divider,
                    1.0,
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let geometry = MacosFrameGeometry::from_request(&request).expect("几何合法");
        let capture = |geometry: MacosFrameGeometry| {
            // SAFETY: `raw` is a live session handle and no other borrow is
            // outstanding at this point.
            let session = unsafe { raw.as_ref() }.expect("session");
            MacosFrameKey::capture(session, geometry)
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
        assert_eq!(moved.revision, baseline.revision, "选区变化不推进 Revision");
        assert_ne!(baseline, moved, "光标移动必须让帧身份改变");

        // 拖动列分隔线既不推进 Revision 也不改变几何。
        let hit =
            begin_table_resize_for_test(raw, 0, 3.1, 0.5, 0.2, 3.1).expect("begin table resize");
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
        assert_eq!(dragged.revision, moved.revision, "拖动不推进 Revision");
        assert_eq!(dragged.geometry, moved.geometry, "拖动不改变平台几何");
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
        let scrolled = MacosFrameGeometry::from_request(&YuStorageFrameGeometry {
            scroll_y: 40.0,
            ..request
        })
        .expect("几何合法");
        assert_ne!(capture(geometry), capture(scrolled), "滚动必须让帧身份改变");

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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-resize-action-{id}.md"));
        fs::write(&path, "| A | B |\n| --- | :---: |\n| 1 | 2 |\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let hit =
            begin_table_resize_for_test(raw, 0, 3.1, 0.5, 0.2, 3.1).expect("begin table resize");
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
        assert_eq!(preview.final_position, 4.0);

        for unknown in [3_u8, 255] {
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
        assert_eq!(preview.final_position, 5.0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_table_resize_gesture_lifecycle_is_revision_bound_and_source_neutral() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
            begin_table_resize_for_test(raw, 0, 3.1, 0.5, 0.2, 3.1).expect("begin table resize");
        assert_eq!(hit.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert_eq!(hit.index, 0);
        assert_eq!(hit.position, 3.0);

        // 已有手势时不得再开一个。
        assert_eq!(
            begin_table_resize_for_test(raw, 0, 3.1, 0.5, 0.2, 3.1),
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
        assert_eq!(preview.initial_position, 3.0);
        assert_eq!(preview.final_position, 4.0);
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

    /// `macos_source_caret` 让平台在不知道 block 归属的情况下取得 caret 几何。
    /// AppKit 的 `firstRect(forCharacterRange:)` 需要它来定位 IME 候选窗：
    /// TextKit 排的是 canonical source，屏幕显示的是投影结果，两者字符位置
    /// 不对应，用默认实现会让候选窗偏离真实插入点（不变量 H3、I1）。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_source_caret_resolves_owning_block_and_is_revision_bound() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                yu_storage_session_macos_source_caret(
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
                yu_storage_session_macos_source_caret(
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
                yu_storage_session_macos_source_caret(
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
                yu_storage_session_macos_source_caret(
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                .inline_projection_for_visual_state()
                .expect("projection");
            projected_utf8(&projection).expect("projected")
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
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                        raw, 0, 14.0, 500.0, 0.0, 240.0, 0, &mut frame,
                    )
                },
                YU_STORAGE_OK
            );
            assert_eq!(frame.published, 1);
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

    /// viewport 配置由 Rust 自己发布，且已经对齐时不得重建。
    ///
    /// 这条路径此前靠十份重复校验守着：平台没送对值就整个调用失败。现在校验
    /// 变成了发布，失败模式也随之变了——发布错了不会报错，只会让整份文档按
    /// 错误的行高与换行宽度排版。因此这里直接断言发布后的配置内容。
    ///
    /// 「已对齐就跳过」不是优化：`set_viewport_config` 会重建 `ViewportLayout`
    /// 并丢掉缓存的 block 高度，那是 J2 定位可见范围的依据。
    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_viewport_config_is_published_by_rust_and_kept_when_aligned() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");

        macos_publish_viewport_config(session, 500.0, metrics).expect("publish");
        let published = session.session.viewport_config().layout();
        assert!((published.max_width() - 500.0).abs() <= f32::EPSILON);
        assert!((published.line_height() - metrics.line_height()).abs() <= f32::EPSILON);
        assert!((published.default_advance() - metrics.default_advance()).abs() <= f32::EPSILON);

        // 用一个可辨认的 estimated_block_height 观察「跳过」是否真的发生。
        let marked = ViewportConfig::new(published, 987.0, 0.0);
        session.session.set_viewport_config(marked).expect("marked");
        macos_publish_viewport_config(session, 500.0, metrics).expect("republish");
        assert!(
            (session.session.viewport_config().estimated_block_height() - 987.0).abs()
                <= f32::EPSILON,
            "配置已经对齐时不得重建 ViewportLayout"
        );

        // 换行宽度变化必须真正重新发布。
        macos_publish_viewport_config(session, 640.0, metrics).expect("publish width");
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
        let viewport = ViewportRect::new(0.0, 240.0);
        for invalid in [0.0_f32, -1.0, f32::NAN, f32::INFINITY] {
            let config = macos_render_host_config(viewport, 14.0, 500.0, 240.0, invalid)
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");

        let mut first = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw, 0, 14.0, 500.0, 0.0, 240.0, 0, &mut first,
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
                        Primitive::Table(table)
                            if table.role() == yu_scene::TablePrimitiveRole::Border
                                && table.bounds().x() > 0.0
                                && table.bounds().x() < 100.0 =>
                        {
                            Some(table.bounds().x())
                        }
                        _ => None,
                    })
                    .next()
            })
            .expect("CoreText table divider");
        let mut accessibility_required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_table_resize_accessibility_dividers(
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
                yu_storage_session_macos_table_resize_accessibility_dividers(
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
        let point_y = metrics.line_height() * 0.5;
        let mut document_hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_table_resize_at_point(
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
                yu_storage_session_macos_table_resize_at_point(
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
                yu_storage_session_macos_table_resize_at_point(
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
                        Primitive::Table(table)
                            if table.role() == yu_scene::TablePrimitiveRole::Border
                                && (table.bounds().x() - (divider + 1.0)).abs() < 0.01
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
                yu_storage_session_macos_table_resize_accessibility_dividers(
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
                yu_storage_session_macos_table_resize_accessibility_dividers(
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
                yu_storage_session_macos_table_resize_at_point(
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
                        Primitive::Table(table)
                            if table.role() == yu_scene::TablePrimitiveRole::Border
                                && (table.bounds().x() - divider).abs() < 0.01
                    )
                })
        );
        assert_eq!(state.session.snapshot().as_str(), source);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_render_host_reuses_state_across_viewport_events() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                    raw, 0, 14.0, 500.0, 0.0, 240.0, 0, &mut first,
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
                    raw, 0, 14.0, 500.0, 0.0, 240.0, 1, &mut stale,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(stale, YuStorageMacosRenderHostSnapshot::default());

        let mut next = YuStorageMacosRenderHostSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_render_host_frame(
                    raw, 1, 14.0, 500.0, 0.0, 240.0, 1, &mut next,
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");

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
                yu_storage_session_macos_shaped_caret_scroll_request(
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
                yu_storage_session_macos_shaped_caret_scroll_request(
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                yu_storage_session_macos_composition_shaped_caret(
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
                yu_storage_session_macos_composition_shaped_caret(
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
    fn ffi_accessibility_semantic_nodes_are_revision_bound_and_source_backed() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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

    #[test]
    fn unified_ffi_command_and_composition_share_revision() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
    fn unified_ffi_insert_text_is_revision_bound_and_returns_range_sync() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
            unsafe { yu_storage_session_copy_selection(raw, 0, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_OK
        );
        let mut selected = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection(
                    raw,
                    0,
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
            unsafe { yu_storage_session_copy_selection(raw, 1, ptr::null_mut(), 0, &mut required) },
            YU_STORAGE_STALE_REVISION
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn unified_ffi_html_selection_is_source_revision_bound() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
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
                yu_storage_session_copy_selection_html(raw, 0, ptr::null_mut(), 0, &mut required)
            },
            YU_STORAGE_OK
        );
        let mut html = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection_html(
                    raw,
                    0,
                    html.as_mut_ptr(),
                    html.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            String::from_utf8(html).expect("HTML UTF-8"),
            "<p><strong>羽</strong></p>"
        );
        assert_eq!(
            unsafe {
                yu_storage_session_copy_selection_html(raw, 1, ptr::null_mut(), 0, &mut required)
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
