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
use yu_core::{ByteOffset, LineIndex, Revision, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{
    ACCESSIBILITY_SEMANTIC_FLAG_ORDERED, ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE,
    AccessibilitySemanticNode, AccessibilitySemanticSnapshot, AccessibilityTextError,
    AccessibilityTextSnapshot, BlockKind, BlockProjection, BlockProjectionKind, CaretAffinity,
    CaretScrollRequest, CommandResult, EditorCommand, EditorDocumentError, EditorKey, ImageSource,
    KeyEvent, KeyModifiers, KeyRouteResult, LayoutConfig, LayoutPoint, LayoutSnapshot, Projection,
    ProjectionBias, SelectionError, SourceSync, TableAlignment, TableCellLayout, TableLayoutHit,
    TableResizeCommit, TableResizeGesture, TableResizeGestureError, TableResizeHit,
    TableResizeTarget, ViewportConfig, ViewportRect, VisualOffset, VisualRunKind,
};
use yu_export::{ExportError, export_clipboard, import_html_fragment};
use yu_markdown::TableCellRange;
use yu_storage::{
    ClosePrompt, CloseRequest, CloseState, DiskState, DocumentEditorSession, ExternalFileState,
    SaveOutcome, StorageError, Utf8Bom,
};
use yu_text::{EditError, TextSnapshot};

#[cfg(target_os = "macos")]
use yu_render::{RenderCommand, RenderPlanBuilder};
#[cfg(target_os = "macos")]
use yu_scene::{Primitive, Rect, Rgba8, SceneBuilder, ViewportBlockGeometry, ViewportSceneInput};
#[cfg(target_os = "macos")]
use yu_workspace::{
    ViewportRenderConfig, assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded,
    viewport_block_background,
};

#[cfg(target_os = "macos")]
use yu_assets::{
    EmbeddedFailureKind, EmbeddedRenderPublication, EmbeddedRenderRequest, EmbeddedRenderer,
    EmbeddedRequestResult, EmbeddedResourceCache, EmbeddedResourceKind, ImageCache,
    ImageFailureKind, ImageIntrinsicPublication, ImagePublication, ImageRequest,
    ImageRequestCandidate, ImageRequestPlan, ImageRequestPriority, ImageRequestResult,
};
#[cfg(target_os = "macos")]
use yu_embedded_math::MathRenderer;
#[cfg(target_os = "macos")]
use yu_font::FontRequest;
#[cfg(target_os = "macos")]
use yu_font::{GlyphAtlas, GlyphAtlasConfig, GlyphRasterKey, GlyphRasterizer};
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

pub const YU_STORAGE_SCENE_PRIMITIVE_BACKGROUND: u8 = 0;
pub const YU_STORAGE_SCENE_PRIMITIVE_TEXT_BOUNDS: u8 = 1;

pub const YU_STORAGE_KEY_CHARACTER: u8 = 0;
pub const YU_STORAGE_KEY_ENTER: u8 = 1;
pub const YU_STORAGE_KEY_TAB: u8 = 2;
pub const YU_STORAGE_KEY_BACKSPACE: u8 = 3;
pub const YU_STORAGE_KEY_DELETE: u8 = 4;
pub const YU_STORAGE_KEY_LEFT: u8 = 5;
pub const YU_STORAGE_KEY_RIGHT: u8 = 6;
pub const YU_STORAGE_KEY_UP: u8 = 7;
pub const YU_STORAGE_KEY_DOWN: u8 = 8;
pub const YU_STORAGE_KEY_ESCAPE: u8 = 9;
pub const YU_STORAGE_KEY_MODIFIER_COMMAND: u8 = 1 << 0;
pub const YU_STORAGE_KEY_MODIFIER_SHIFT: u8 = 1 << 1;
pub const YU_STORAGE_KEY_MODIFIER_CONTROL: u8 = 1 << 2;
pub const YU_STORAGE_KEY_MODIFIER_OPTION: u8 = 1 << 3;
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageSelection {
    pub revision: u64,
    pub start_utf16: u64,
    pub end_utf16: u64,
    pub affinity: u8,
}

/// Revision-bound selection endpoints. `anchor_utf16` and `focus_utf16`
/// preserve the direction of a native drag, while `start_utf16`/`end_utf16`
/// in [`YuStorageSelection`] intentionally remain the ordered range used by
/// existing TextKit and Accessibility callers.
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

/// Revision-bound source selection projected into visual UTF-16 coordinates.
/// Non-collapsed selections map their source start/end with the outer
/// projection boundaries, so hidden Markdown delimiters are not accidentally
/// reintroduced into the visual range. Collapsed selections retain the caller
/// affinity and should be handled as a caret by native hosts.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageProjectionSelection {
    pub revision: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub visual_start_utf16: u64,
    pub visual_end_utf16: u64,
    pub round_trip_source_start_utf16: u64,
    pub round_trip_source_end_utf16: u64,
    pub affinity: u8,
}

/// Revision-bound reverse caret mapping for a native visual mirror. The input
/// visual coordinate and the returned source coordinate use UTF-16 units;
/// `round_trip_visual_utf16` proves the source boundary maps back under the
/// requested affinity.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageProjectionSourceCaret {
    pub revision: u64,
    pub visual_utf16: u64,
    pub source_utf16: u64,
    pub round_trip_visual_utf16: u64,
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

/// Revision- and composition-generation-bound caret mapping for the active
/// marked-text projection. `visual_utf16` and the visual selection are owned
/// projected-stream coordinates; source remains canonical UTF-16.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageCompositionCaret {
    pub revision: u64,
    pub generation: u64,
    pub source_utf16: u64,
    pub visual_utf16: u64,
    pub round_trip_source_utf16: u64,
    pub visual_selection_start_utf16: u64,
    pub visual_selection_end_utf16: u64,
    pub affinity: u8,
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

/// Revision- and composition-generation-bound CoreText-shaped point hit-test
/// for the transient marked-text projection. Coordinates are document-space;
/// source and visual offsets are mapped through the same full transient
/// projection, so a native host never has to reconstruct preedit offsets.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageCompositionProjectionHit {
    pub revision: u64,
    pub generation: u64,
    pub source_utf16: u64,
    pub block_index: u64,
    pub visual_utf16: u64,
    pub round_trip_source_utf16: u64,
    pub line: u64,
    pub x: f32,
    pub y: f32,
    pub visual_selection_start_utf16: u64,
    pub visual_selection_end_utf16: u64,
    pub visual_replacement_start_utf16: u64,
    pub visual_replacement_end_utf16: u64,
    pub affinity: u8,
}

/// Revision-bound metadata for one parser-owned block projection. The visual
/// bytes are returned by the companion query; lengths are included here so a
/// native host can validate its allocation and its UTF-16 layout without
/// reparsing Markdown.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageProjectionBlock {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub visual_utf8_length: u64,
    pub visual_utf16_length: u64,
    pub kind: u8,
    pub projection_kind: u8,
}

/// One parser-owned GFM table cell range. `row = 0` is the header, `row = 1`
/// is the delimiter row, and body rows start at `row = 2`. All offsets are
/// UTF-16 positions in the revision supplied to the query.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageTableCellRange {
    pub row: u64,
    pub column: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
}

/// Revision-bound geometry for one visible source-backed table cell. `row = 0`
/// is the header and body rows start at `row = 1`; the Markdown delimiter row
/// is intentionally absent from this visible layout list.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageTableLayoutCell {
    pub revision: u64,
    pub block_index: u64,
    pub row: u64,
    pub column: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub alignment: u8,
}

/// Revision-bound hit-test result for a visible table cell. The point supplied
/// by the native caller remains in its local table coordinate system; the
/// result returns the hit cell's bounds and source range.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageTableCellHit {
    pub revision: u64,
    pub block_index: u64,
    pub row: u64,
    pub column: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
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

/// Revision-bound layout metadata for one parser-owned block. `width` and
/// `height` are local layout points; `shaped` distinguishes deterministic
/// metrics from macOS CoreText output.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageBlockLayout {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub visual_utf16_length: u64,
    pub line_count: u64,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
    pub default_advance: f32,
    pub kind: u8,
    pub projection_kind: u8,
    pub shaped: u8,
}

/// Revision-bound CoreText metrics for configuring an empty or non-empty
/// viewport. This is intentionally independent of parser block metadata so a
/// native host can initialize a surface before the Markdown document has a
/// parser-owned block.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageMacosFontMetrics {
    pub revision: u64,
    pub size: f32,
    pub line_height: f32,
    pub default_advance: f32,
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

/// Revision-bound metadata for one block returned by a shaped viewport query.
/// `y` and `height` are document-space points; source ranges are UTF-16 units.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageShapedViewportBlock {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub y: f32,
    pub height: f32,
    pub measured: u8,
    pub kind: u8,
}

/// Owned metadata for one shaped viewport snapshot. Blocks are returned via
/// the count/fill ABI and never retain Rust references across the boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageShapedViewportSnapshot {
    pub revision: u64,
    pub block_start: u64,
    pub block_end: u64,
    pub content_height: f32,
    /// Document-space viewport inputs used for this snapshot. `scroll_y` is
    /// the requested native scroll offset; callers clamp it to `max_scroll_y`
    /// when converting viewport-local points back to document coordinates.
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub max_scroll_y: f32,
}

/// Owned, revision- and composition-generation-bound visual decoration
/// snapshot. Selection rectangles are returned by the companion count/fill
/// query; their coordinates are document-space and therefore must be offset
/// by the native scroll position before painting in a viewport sibling.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageMacosVisualDecorationSnapshot {
    pub revision: u64,
    pub composition_generation: u64,
    pub selection_count: u64,
    pub caret_present: u8,
    pub content_height: f32,
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub max_scroll_y: f32,
    pub viewport_width: f32,
}

/// One Rust-shaped selection rectangle. Coordinates are in the document
/// layout space of the requested viewport; no AppKit/TextKit object crosses
/// the C ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageMacosVisualDecorationRect {
    pub revision: u64,
    pub block_index: u64,
    pub line_index: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub kind: u8,
}

/// One Rust-shaped caret geometry record. The caret is document-space and is
/// present only when its block is part of the requested visible window.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageMacosVisualDecorationCaret {
    pub revision: u64,
    pub block_index: u64,
    pub line_index: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub affinity: u8,
    pub present: u8,
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

/// A revision-bound, owned scene snapshot for the native visual-rendering
/// diagnostic bridge. The scene is assembled by Rust's `ViewportSceneInput`
/// and `SceneBuilder`; no parser, layout or native object crosses the ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageVisualSceneSnapshot {
    pub revision: u64,
    pub block_start: u64,
    pub block_end: u64,
    pub primitive_count: u64,
    pub content_height: f32,
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub max_scroll_y: f32,
    pub viewport_width: f32,
}

/// One owned, revision-bound scene primitive. The first implementation uses
/// rectangle primitives for block backgrounds and text ink bounds; glyph and
/// image payloads will be added only after this native ownership boundary is
/// proven. Source ranges remain UTF-16 for direct AppKit validation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageVisualScenePrimitive {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub kind: u8,
}

pub const YU_STORAGE_RENDER_COMMAND_FILL_RECT: u8 = 0;
pub const YU_STORAGE_RENDER_COMMAND_GLYPH: u8 = 1;
pub const YU_STORAGE_RENDER_COMMAND_IMAGE: u8 = 2;
pub const YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG: u8 = 3;
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

/// Source-backed metadata for one image in the current Markdown Revision.
/// Destination and reference values are UTF-16 source ranges; native code can
/// fetch the actual bytes through the existing expected-Revision source range
/// API and hand them to an async platform decoder.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageVisualImage {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub label_start_utf16: u64,
    pub label_end_utf16: u64,
    pub destination_start_utf16: u64,
    pub destination_end_utf16: u64,
    pub reference_start_utf16: u64,
    pub reference_end_utf16: u64,
    pub resource_fingerprint: u64,
    pub kind: u8,
    /// Resource readiness for the current viewport host. This is deliberately
    /// appended to preserve the existing C ABI layout for older fields.
    pub resource_status: u8,
}

/// Source-backed metadata for an embedded Math or Mermaid fenced block.
/// Renderer state is intentionally separate from the Markdown source: native
/// code can keep the complete source range visible until a future renderer
/// publishes a revision-bound resource.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageVisualEmbeddedResource {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub info_start_utf16: u64,
    pub info_end_utf16: u64,
    pub content_start_utf16: u64,
    pub content_end_utf16: u64,
    pub resource_fingerprint: u64,
    pub kind: u8,
    pub resource_status: u8,
}

/// Owned metadata for one backend-neutral render-plan publication. Atlas
/// pixels remain an owned Rust-side upload payload; this ABI exposes their
/// page identity/fingerprint so native diagnostics can validate publication
/// without retaining Rust allocations.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageVisualRenderPlanSnapshot {
    pub revision: u64,
    /// Monotonic transient IME identity captured with this count/fill
    /// publication. Native callers must use the same value for both calls.
    pub composition_generation: u64,
    pub block_start: u64,
    pub block_end: u64,
    pub command_count: u64,
    pub upload_count: u64,
    pub damage_count: u64,
    pub content_height: f32,
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub max_scroll_y: f32,
    pub viewport_width: f32,
    /// Appended diagnostics for the embedded SVG scene/render boundary.
    pub embedded_command_count: u64,
    pub embedded_upload_count: u64,
    pub embedded_upload_bytes: u64,
}

/// One owned render command. Glyph atlas placement, baseline origin, metrics,
/// source block range and command bounds are copied from the Rust
/// `RenderPlan`; solid block fills use the same bounds/color fields and set
/// atlas values to their zero/none defaults. No scene or atlas reference
/// crosses the ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageVisualRenderCommand {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub kind: u8,
    pub page: u32,
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance_x: f32,
    pub bounds_x: f32,
    pub bounds_y: f32,
    pub bounds_width: f32,
    pub bounds_height: f32,
    pub color_rgba: u32,
    pub resource: u64,
    /// Appended embedded-resource identity and intrinsic dimensions. Existing
    /// native callers can ignore these fields; a future SVG backend can use
    /// them to match a render-plan upload to its command.
    pub embedded_generation: u64,
    pub embedded_kind: u8,
    pub embedded_width: u32,
    pub embedded_height: u32,
}

/// One owned atlas-page publication record. The corresponding alpha bytes are
/// retained only by Rust's `RenderPlan`/renderer pipeline for this diagnostic
/// call; the fingerprint makes page deduplication observable at the boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageVisualRenderPage {
    pub revision: u64,
    pub page: u32,
    pub width: u32,
    pub height: u32,
    pub fingerprint: u64,
}

/// One owned damage rectangle from the same render plan publication.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageVisualRenderDamage {
    pub revision: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

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
    /// Bitset of `RenderCommand` kinds present in this publication. The
    /// field is appended so older scalar offsets remain stable for native
    /// diagnostics while newer hosts can reject commands they do not know
    /// how to draw.
    pub command_kind_mask: u64,
    /// Bitset of parser-owned block tags present in the current viewport.
    /// Unknown tags are represented by the high sentinel bit.
    pub block_kind_mask: u64,
}

/// Scalar result from the opt-in real CAMetalLayer submit bridge. The view,
/// layer, renderer, atlas and command queue remain owned by the synchronous
/// Rust call; only lifecycle metadata crosses the ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// Same command-kind capability bitset as the retained host snapshot.
    /// Native presentation must treat unknown bits as a fallback condition.
    pub command_kind_mask: u64,
    /// Same viewport block-tag summary as the retained host snapshot.
    pub block_kind_mask: u64,
}

/// One glyph primitive copied from the retained scene produced by the
/// persistent macOS render host. Atlas pixels stay in Rust; this ABI exposes
/// only the validated placement and source-backed block metadata needed by an
/// opt-in native scene consumer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageVisualSceneGlyph {
    pub revision: u64,
    pub block_index: u64,
    pub source_start_utf16: u64,
    pub source_end_utf16: u64,
    pub page: u32,
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance_x: f32,
    pub bounds_x: f32,
    pub bounds_y: f32,
    pub bounds_width: f32,
    pub bounds_height: f32,
    pub color_rgba: u32,
}

/// Header for a retained-scene glyph publication. It shares the same host
/// Revision/frame/surface identity as `YuStorageMacosRenderHostSnapshot`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YuStorageVisualSceneGlyphSnapshot {
    pub revision: u64,
    pub composition_generation: u64,
    pub frame_revision: u64,
    pub surface_generation: u64,
    pub frame_serial: u64,
    pub block_start: u64,
    pub block_end: u64,
    pub glyph_count: u64,
    pub content_height: f32,
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub max_scroll_y: f32,
    pub viewport_width: f32,
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

/// One source-backed semantic node for a VoiceOver/native accessibility tree.
/// `index` and `parent` are valid only for the same `revision`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuStorageAccessibilityNode {
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

    fn publication_for(
        &mut self,
        request: EmbeddedRenderRequest,
        revision: Revision,
    ) -> Result<Option<EmbeddedRenderPublication>, i32> {
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

fn external_file_state_from_ffi(value: u8) -> Result<ExternalFileState, i32> {
    match value {
        YU_STORAGE_DISK_CHANGED => Ok(ExternalFileState::Changed),
        YU_STORAGE_DISK_MISSING => Ok(ExternalFileState::Missing),
        _ => Err(YU_STORAGE_INVALID_STATE),
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

/// Validates the header returned by a count query before a native caller
/// performs the matching fill query.  Revision alone is insufficient while
/// marked text is active because composition updates deliberately keep the
/// canonical Revision unchanged.  The first count query passes a zeroed
/// header; only a non-zero array capacity is a fill operation that must match
/// the prior header identity.
#[cfg(target_os = "macos")]
fn validate_visual_fill_identity(
    session: &DocumentEditorSession,
    expected_revision: u64,
    prior_revision: u64,
    prior_generation: u64,
) -> Result<(), i32> {
    if prior_revision != expected_revision {
        return Err(YU_STORAGE_STALE_REVISION);
    }
    if prior_generation != session.composition_generation() {
        return Err(YU_STORAGE_STALE_COMPOSITION);
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

fn selection_from_ffi(
    session: &DocumentEditorSession,
    start_utf16: u64,
    end_utf16: u64,
    affinity: u8,
) -> Result<yu_editor::EditorSelection, i32> {
    let affinity = caret_affinity_from_ffi(affinity)?;
    let range = Utf16Range::new(Utf16Offset::new(start_utf16), Utf16Offset::new(end_utf16))
        .ok_or(YU_STORAGE_INVALID_SELECTION)?;
    let snapshot = session.snapshot();
    let start = snapshot
        .byte_offset_for_utf16(range.start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let end = snapshot
        .byte_offset_for_utf16(range.end())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    yu_editor::EditorSelection::range(&snapshot, start, end, affinity)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)
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

fn selection_output(session: &DocumentEditorSession, output: *mut YuStorageSelection) -> i32 {
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let snapshot = session.snapshot();
    let selection = session.selection();
    let range = match selection.utf16_range(&snapshot) {
        Ok(range) => range,
        Err(error) => return status_from_editor_error(EditorDocumentError::Selection(error)),
    };
    // SAFETY: output is checked above and belongs to the caller.
    unsafe {
        *output = YuStorageSelection {
            revision: session.revision().get(),
            start_utf16: range.start().get(),
            end_utf16: range.end().get(),
            affinity: match selection.affinity() {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        };
    }
    YU_STORAGE_OK
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

fn accessibility_semantic_node_output(
    node: AccessibilitySemanticNode,
) -> YuStorageAccessibilityNode {
    let source = node.source_range().range();
    let label = node.label_range().range();
    YuStorageAccessibilityNode {
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
    }
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

fn write_accessibility_nodes(
    nodes: &[AccessibilitySemanticNode],
    output: *mut YuStorageAccessibilityNode,
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
        .map(accessibility_semantic_node_output)
        .collect::<Vec<_>>();
    // SAFETY: capacity was checked against the number of converted nodes, and
    // the native caller supplied writable storage for that many values.
    unsafe {
        ptr::copy_nonoverlapping(converted.as_ptr(), output, converted.len());
    }
    YU_STORAGE_OK
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

fn key_from_ffi(kind: u8, value: u32) -> Result<EditorKey, i32> {
    match kind {
        YU_STORAGE_KEY_CHARACTER => char::from_u32(value)
            .map(EditorKey::Character)
            .ok_or(YU_STORAGE_INVALID_KEY),
        YU_STORAGE_KEY_ENTER => Ok(EditorKey::Enter),
        YU_STORAGE_KEY_TAB => Ok(EditorKey::Tab),
        YU_STORAGE_KEY_BACKSPACE => Ok(EditorKey::Backspace),
        YU_STORAGE_KEY_DELETE => Ok(EditorKey::Delete),
        YU_STORAGE_KEY_LEFT => Ok(EditorKey::Left),
        YU_STORAGE_KEY_RIGHT => Ok(EditorKey::Right),
        YU_STORAGE_KEY_UP => Ok(EditorKey::Up),
        YU_STORAGE_KEY_DOWN => Ok(EditorKey::Down),
        YU_STORAGE_KEY_ESCAPE => Ok(EditorKey::Escape),
        _ => Err(YU_STORAGE_INVALID_KEY),
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

fn projection_kind_tag(projection: &BlockProjection) -> u8 {
    match projection.kind() {
        BlockProjectionKind::Inline => YU_STORAGE_PROJECTION_INLINE,
        BlockProjectionKind::Heading => YU_STORAGE_PROJECTION_HEADING,
        BlockProjectionKind::BlockQuote => YU_STORAGE_PROJECTION_BLOCK_QUOTE,
        BlockProjectionKind::List => YU_STORAGE_PROJECTION_LIST,
        BlockProjectionKind::Table => YU_STORAGE_PROJECTION_TABLE,
        BlockProjectionKind::FencedCode => YU_STORAGE_PROJECTION_FENCED_CODE,
        BlockProjectionKind::ReferenceDefinition => YU_STORAGE_PROJECTION_REFERENCE_DEFINITION,
        BlockProjectionKind::TaskList => YU_STORAGE_PROJECTION_TASK_LIST,
    }
}

fn table_cell_metadata(
    snapshot: &TextSnapshot,
    row: usize,
    column: usize,
    cell: TableCellRange,
) -> Result<YuStorageTableCellRange, i32> {
    let start = ByteOffset::try_from(cell.start()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let end = ByteOffset::try_from(cell.end()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let source_start_utf16 = snapshot
        .utf16_offset(start)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    let source_end_utf16 = snapshot
        .utf16_offset(end)
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .get();
    Ok(YuStorageTableCellRange {
        row: u64::try_from(row).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        column: u64::try_from(column).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        source_start_utf16,
        source_end_utf16,
    })
}

fn table_cell_ranges(
    snapshot: &TextSnapshot,
    projection: &BlockProjection,
) -> Result<Vec<YuStorageTableCellRange>, i32> {
    let BlockProjection::Table(table) = projection else {
        return Err(YU_STORAGE_INVALID_SELECTION);
    };
    let metadata = table.table();
    let capacity = metadata
        .header()
        .len()
        .saturating_mul(metadata.rows().len().saturating_add(2));
    let mut encoded = Vec::with_capacity(capacity);
    for (column, cell) in metadata.header().iter().copied().enumerate() {
        encoded.push(table_cell_metadata(snapshot, 0, column, cell)?);
    }
    for (column, cell) in metadata.delimiter().iter().copied().enumerate() {
        encoded.push(table_cell_metadata(snapshot, 1, column, cell)?);
    }
    for (body_index, row) in metadata.rows().iter().enumerate() {
        for (column, cell) in row.iter().copied().enumerate() {
            encoded.push(table_cell_metadata(
                snapshot,
                body_index.saturating_add(2),
                column,
                cell,
            )?);
        }
    }
    Ok(encoded)
}

fn table_alignment_tag(alignment: TableAlignment) -> u8 {
    match alignment {
        TableAlignment::Default => YU_STORAGE_TABLE_ALIGNMENT_DEFAULT,
        TableAlignment::Left => YU_STORAGE_TABLE_ALIGNMENT_LEFT,
        TableAlignment::Center => YU_STORAGE_TABLE_ALIGNMENT_CENTER,
        TableAlignment::Right => YU_STORAGE_TABLE_ALIGNMENT_RIGHT,
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

fn table_layout_cell_metadata(
    snapshot: &TextSnapshot,
    revision: u64,
    block_index: u64,
    cell: TableCellLayout,
) -> Result<YuStorageTableLayoutCell, i32> {
    let (source_start_utf16, source_end_utf16) = table_source_utf16_range(snapshot, cell.source())?;
    Ok(YuStorageTableLayoutCell {
        revision,
        block_index,
        row: u64::try_from(cell.row()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        column: u64::try_from(cell.column()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        source_start_utf16,
        source_end_utf16,
        x: cell.bounds().x(),
        y: cell.bounds().y(),
        width: cell.bounds().width(),
        height: cell.bounds().height(),
        alignment: table_alignment_tag(cell.alignment()),
    })
}

fn table_layout_cells_metadata(
    snapshot: &TextSnapshot,
    revision: u64,
    block_index: u64,
    table: &yu_editor::TableLayoutSnapshot,
) -> Result<Vec<YuStorageTableLayoutCell>, i32> {
    table
        .cells()
        .iter()
        .copied()
        .map(|cell| table_layout_cell_metadata(snapshot, revision, block_index, cell))
        .collect()
}

fn table_layout_hit_metadata(
    snapshot: &TextSnapshot,
    revision: u64,
    block_index: u64,
    hit: TableLayoutHit,
) -> Result<YuStorageTableCellHit, i32> {
    let (source_start_utf16, source_end_utf16) = table_source_utf16_range(snapshot, hit.source())?;
    Ok(YuStorageTableCellHit {
        revision,
        block_index,
        row: u64::try_from(hit.row()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        column: u64::try_from(hit.column()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        source_start_utf16,
        source_end_utf16,
        x: hit.bounds().x(),
        y: hit.bounds().y(),
        width: hit.bounds().width(),
        height: hit.bounds().height(),
    })
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

fn viewport_block_kind(kind: BlockKind) -> u8 {
    kind.viewport_tag()
}

fn block_layout_metadata(
    session: &mut DocumentEditorSession,
    block_index: usize,
    layout: &LayoutSnapshot,
    line_height: f32,
    default_advance: f32,
    shaped: u8,
) -> Result<YuStorageBlockLayout, i32> {
    if layout.revision() != session.revision() {
        return Err(YU_STORAGE_STALE_REVISION);
    }
    let Some((source_range, kind)) = session.block_metadata(block_index) else {
        return Err(YU_STORAGE_INVALID_SELECTION);
    };
    let projection = session
        .block_projection(block_index)
        .map_err(storage_status)?;
    let projected = projected_utf8(layout.projection())?;
    let visual_utf16_length = visual_utf16_offset(&projected, layout.visual_len())?;
    let line_count = u64::try_from(layout.lines().len())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
        .max(1);
    let width = layout
        .lines()
        .iter()
        .map(|line| line.width())
        .fold(0.0_f32, f32::max);
    let height = line_height * line_count as f32;
    if !width.is_finite()
        || !height.is_finite()
        || !line_height.is_finite()
        || line_height <= 0.0
        || !default_advance.is_finite()
        || default_advance <= 0.0
    {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    Ok(YuStorageBlockLayout {
        revision: session.revision().get(),
        block_index: u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        source_start_utf16: session
            .snapshot()
            .utf16_offset(source_range.start())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get(),
        source_end_utf16: session
            .snapshot()
            .utf16_offset(source_range.end())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get(),
        visual_utf16_length,
        line_count,
        width,
        height,
        line_height,
        default_advance,
        kind,
        projection_kind: projection_kind_tag(&projection),
        shaped,
    })
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

/// # Safety
///
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_selection(
    session: *const YuStorageSession,
    output: *mut YuStorageSelection,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    selection_output(&session.session, output)
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

/// # Safety
///
/// `session` must be a live handle. The expected revision and UTF-16 range
/// must describe a valid source selection; `affinity` must be a known value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_selection(
    session: *mut YuStorageSession,
    expected_revision: u64,
    start_utf16: u64,
    end_utf16: u64,
    affinity: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let selection = match selection_from_ffi(&session.session, start_utf16, end_utf16, affinity) {
        Ok(selection) => selection,
        Err(status) => return status,
    };
    session
        .session
        .set_selection(selection)
        .map_or_else(storage_status, |_| YU_STORAGE_OK)
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
        let viewport_config = session.session.viewport_config().layout();
        if (viewport_config.max_width() - max_width).abs() > 0.05
            || (viewport_config.line_height() - metrics.line_height()).abs() > 0.05
            || (viewport_config.default_advance() - metrics.default_advance()).abs() > 0.05
        {
            return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
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

/// # Safety
///
/// `session` must be a live handle and `output` must be writable when a key
/// is handled. Printable text is intentionally left to native text input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_route_key(
    session: *mut YuStorageSession,
    key_kind: u8,
    key: u32,
    modifiers: u8,
    output: *mut YuStorageCommandResult,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let key = match key_from_ffi(key_kind, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    let event = KeyEvent::new(key, KeyModifiers::from_bits(modifiers));
    let route = match session.session.route_key(event) {
        Ok(route) => route,
        Err(error) => return storage_status(error),
    };
    let KeyRouteResult::Executed(result) = route else {
        return YU_STORAGE_KEY_UNHANDLED;
    };
    command_result_output(&session.session, result, output)
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
/// `session` must be null or a live handle; `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_path_length(
    session: *const YuStorageSession,
    output: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let path = session.session.path().to_string_lossy();
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe { *output = path.len() };
    YU_STORAGE_OK
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
/// `session` must be null or a live handle and `output` must be writable when
/// non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_source_length(
    session: *const YuStorageSession,
    output: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let source = session.session.snapshot();
    // SAFETY: `output` was checked and belongs to the caller.
    unsafe { *output = source.as_str().len() };
    YU_STORAGE_OK
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

/// Returns the current revision's source-backed inline projection as UTF-8.
/// Hidden Markdown delimiter runs are omitted; visible text and parser-owned
/// line-break runs retain their source order. The projection is built through
/// the editor cache owned by this same session and is never treated as
/// canonical source.
///
/// # Safety
/// `session` must be a live handle. `expected_revision` must match the current
/// session revision. `written` must be writable; `output` must provide
/// `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projected_source(
    session: *mut YuStorageSession,
    expected_revision: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let projection = match session.session.inline_projection() {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let projected = match projected_utf8(&projection) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
    write_bytes(projected.as_bytes(), output, capacity, written)
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
    let projection = match session.session.inline_projection() {
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

/// Maps a canonical source selection through the current inline projection.
/// Non-collapsed ranges use `Before` for the start and `After` for the end so
/// hidden Markdown delimiters do not become visual selection content. A
/// collapsed range is a caret and keeps the requested affinity.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projection_selection(
    session: *mut YuStorageSession,
    expected_revision: u64,
    source_start_utf16: u64,
    source_end_utf16: u64,
    affinity: u8,
    output: *mut YuStorageProjectionSelection,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageProjectionSelection::default() };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let affinity = match caret_affinity_from_ffi(affinity) {
        Ok(affinity) => affinity,
        Err(status) => return status,
    };
    if source_start_utf16 > source_end_utf16 {
        return YU_STORAGE_INVALID_SELECTION;
    }
    let snapshot = session.session.snapshot();
    let source_start = match snapshot.byte_offset_for_utf16(Utf16Offset::new(source_start_utf16)) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let source_end = match snapshot.byte_offset_for_utf16(Utf16Offset::new(source_end_utf16)) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let projection = match session.session.inline_projection() {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let (start_bias, end_bias) = if source_start_utf16 == source_end_utf16 {
        let bias = projection_bias_from_affinity(affinity);
        (bias, bias)
    } else {
        (ProjectionBias::Before, ProjectionBias::After)
    };
    let visual_start = match projection.source_to_visual(source_start, start_bias) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let visual_end = match projection.source_to_visual(source_end, end_bias) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_start = match projection.visual_to_source(visual_start, start_bias) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_end = match projection.visual_to_source(visual_end, end_bias) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let projected = match projected_utf8(&projection) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
    let visual_start_utf16 = match visual_utf16_offset(&projected, visual_start) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    let visual_end_utf16 = match visual_utf16_offset(&projected, visual_end) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    let round_trip_source_start_utf16 = match snapshot.utf16_offset(round_trip_start) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_source_end_utf16 = match snapshot.utf16_offset(round_trip_end) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe {
        *output = YuStorageProjectionSelection {
            revision: session.session.revision().get(),
            source_start_utf16,
            source_end_utf16,
            visual_start_utf16,
            visual_end_utf16,
            round_trip_source_start_utf16,
            round_trip_source_end_utf16,
            affinity: affinity_to_ffi(affinity),
        };
    }
    YU_STORAGE_OK
}

/// Maps one visual UTF-16 caret from a native TextKit mirror back to the
/// canonical source. The visual stream is Rust-owned projected text; Swift
/// must not infer hidden Markdown delimiter ranges itself.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projection_source_caret(
    session: *mut YuStorageSession,
    expected_revision: u64,
    visual_utf16: u64,
    affinity: u8,
    output: *mut YuStorageProjectionSourceCaret,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageProjectionSourceCaret::default() };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let affinity = match caret_affinity_from_ffi(affinity) {
        Ok(affinity) => affinity,
        Err(status) => return status,
    };
    let projection = match session.session.inline_projection() {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let projected = match projected_utf8(&projection) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
    let visual_byte = match utf16_byte_offset(&projected, visual_utf16) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    let visual = VisualOffset::new(match u64::try_from(visual_byte) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    });
    let bias = projection_bias_from_affinity(affinity);
    let source = match projection.visual_to_source(visual, bias) {
        Ok(source) => source,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_visual = match projection.source_to_visual(source, bias) {
        Ok(visual) => visual,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let snapshot = session.session.snapshot();
    let source_utf16 = match snapshot.utf16_offset(source) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_visual_utf16 = match visual_utf16_offset(&projected, round_trip_visual) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    // SAFETY: output was checked above and belongs to the caller.
    unsafe {
        *output = YuStorageProjectionSourceCaret {
            revision: session.session.revision().get(),
            visual_utf16,
            source_utf16,
            round_trip_visual_utf16,
            affinity: affinity_to_ffi(affinity),
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
    let projection = match session.session.inline_projection() {
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

/// Resolves a projection-local point through the current full-source metrics
/// projection. The layout configuration is explicit in the ABI so the native
/// host cannot silently apply a second wrapping/line-height policy. Returned
/// coordinates are snapped caret coordinates in the same layout space.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projection_hit_test(
    session: *mut YuStorageSession,
    expected_revision: u64,
    point_x: f32,
    point_y: f32,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
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
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let config = LayoutConfig::new(max_width, line_height).with_default_advance(default_advance);
    let layout = match session.session.inline_layout(config) {
        Ok(layout) => layout,
        Err(error) => return storage_status(error),
    };
    let hit = match layout.hit_test(LayoutPoint::new(point_x, point_y)) {
        Ok(hit) => hit,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let snapshot = session.session.snapshot();
    let projected = match projected_utf8(layout.projection()) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
    let visual_utf16 = match visual_utf16_offset(&projected, hit.visual()) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    let round_trip_source = match layout
        .projection()
        .visual_to_source(hit.visual(), hit.bias())
    {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let source_utf16 = match snapshot.utf16_offset(hit.source()) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_source_utf16 = match snapshot.utf16_offset(round_trip_source) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let (image_source_start_utf16, image_source_end_utf16) =
        match image_utf16_range(&snapshot, hit.image()) {
            Ok(range) => range,
            Err(status) => return status,
        };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe {
        *output = YuStorageProjectionHit {
            revision: session.session.revision().get(),
            source_utf16,
            visual_utf16,
            round_trip_source_utf16,
            image_source_start_utf16,
            image_source_end_utf16,
            line: hit.line() as u64,
            x: hit.point().x(),
            y: hit.point().y(),
            affinity: affinity_to_ffi(match hit.bias() {
                ProjectionBias::Before => CaretAffinity::Upstream,
                ProjectionBias::After => CaretAffinity::Downstream,
            }),
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
        let viewport_config = session.session.viewport_config();
        let published = viewport_config.layout();
        if (published.max_width() - max_width).abs() > 0.05
            || (published.line_height() - metrics.line_height()).abs() > 0.05
            || (published.default_advance() - metrics.default_advance()).abs() > 0.05
        {
            return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
        }

        let query_y = point_y.max(0.0);
        let viewport = ViewportRect::new(query_y, metrics.line_height());
        let snapshot = {
            let document = session.session.document_mut().editor_mut();
            match if document.composition().is_some() {
                document.visible_blocks_with_composition_and_shaper(viewport, &shaper)
            } else {
                document.visible_blocks_with_shaper(viewport, &shaper)
            } {
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
            let composition_blocks = document.composition_block_range();
            match if composition_blocks
                .as_ref()
                .is_some_and(|span| span.contains(&block.index()))
            {
                document.block_layout_with_composition_and_shaper(
                    block.index(),
                    layout_config,
                    &shaper,
                )
            } else {
                document
                    .block_layout_with_shaper(block.index(), layout_config, &shaper)
                    .cloned()
            } {
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
        let projection = match session.session.inline_projection() {
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

/// Resolves a document-space point through the current CoreText-shaped
/// transient composition layout. Unlike the canonical projection hit-test,
/// this endpoint is bound to both the source Revision and the composition
/// generation, and maps the hit through the full transient projection.
/// `x`/`y` are document-space coordinates and the returned visual ranges are
/// UTF-16 offsets in that same transient projected stream.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_composition_projection_hit_test(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    point_x: f32,
    point_y: f32,
    size: f32,
    max_width: f32,
    output: *mut YuStorageCompositionProjectionHit,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageCompositionProjectionHit::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            expected_generation,
            point_x,
            point_y,
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
        let viewport_config = session.session.viewport_config();
        let published = viewport_config.layout();
        if (published.max_width() - max_width).abs() > 0.05
            || (published.line_height() - metrics.line_height()).abs() > 0.05
            || (published.default_advance() - metrics.default_advance()).abs() > 0.05
        {
            return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
        }

        let query_y = point_y.max(0.0);
        let viewport = ViewportRect::new(query_y, metrics.line_height());
        let viewport_snapshot = {
            let document = session.session.document_mut().editor_mut();
            match document.visible_blocks_with_composition_and_shaper(viewport, &shaper) {
                Ok(snapshot) => snapshot,
                Err(error) => return status_from_editor_error(error),
            }
        };
        let mut selected = None;
        let mut best_distance = f32::INFINITY;
        for block in viewport_snapshot.blocks() {
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
            let composition_blocks = document.composition_block_range();
            if composition_blocks
                .as_ref()
                .is_some_and(|span| span.contains(&block.index()))
            {
                document
                    .block_layout_with_composition_and_shaper(block.index(), layout_config, &shaper)
                    .map_err(status_from_editor_error)
            } else {
                document
                    .block_layout_with_shaper(block.index(), layout_config, &shaper)
                    .cloned()
                    .map_err(status_from_editor_error)
            }
        };
        let layout = match layout {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        if layout.lines().is_empty() {
            return YU_STORAGE_INVALID_SELECTION;
        }
        let local_y = (query_y - block.y()).max(0.0);
        let hit = match layout.hit_test(LayoutPoint::new(point_x, local_y)) {
            Ok(hit) => hit,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let projection = match composition_projection(&mut session.session) {
            Ok(projection) => projection,
            Err(status) => return status,
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
        let (visual_selection_start_utf16, visual_selection_end_utf16) =
            match composition_visual_selection_utf16(&projection, &projected) {
                Ok(selection) => selection,
                Err(status) => return status,
            };
        let (visual_replacement_start_utf16, visual_replacement_end_utf16) =
            match composition_visual_replacement_utf16(&projection, &projected) {
                Ok(replacement) => replacement,
                Err(status) => return status,
            };
        let point = hit.point();
        let document_y = block.y() + point.y();
        if !point.x().is_finite() || !document_y.is_finite() {
            return YU_STORAGE_EDITOR_ERROR;
        }
        let line_base = (block.y() / metrics.line_height()).floor().max(0.0) as u64;
        let block_index = match u64::try_from(block.index()) {
            Ok(index) => index,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe {
            *output = YuStorageCompositionProjectionHit {
                revision: session.session.revision().get(),
                generation: session.session.composition_generation(),
                source_utf16,
                block_index,
                visual_utf16,
                round_trip_source_utf16,
                line: line_base.saturating_add(hit.line() as u64),
                x: point.x(),
                y: document_y,
                visual_selection_start_utf16,
                visual_selection_end_utf16,
                visual_replacement_start_utf16,
                visual_replacement_end_utf16,
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

/// Copies the current transient projected UTF-8 stream using the same
/// two-call count/fill contract as source snapshots. Both canonical Revision
/// and transient composition generation are validated before projection work.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_copy_composition_projection(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let (_, projected) = match composition_projection_metadata(&mut session.session) {
        Ok(value) => value,
        Err(status) => return status,
    };
    write_bytes(projected.as_bytes(), output, capacity, written)
}

/// Resolves the active marked-text caret through the transient projection.
/// The input source boundary is validated against canonical source; the
/// visual caret is the active end of the preedit selection, matching AppKit's
/// marked-text insertion point. Both visual selection and round-trip source
/// are returned in owned UTF-16/scalar form.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_composition_caret(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    source_utf16: u64,
    affinity: u8,
    output: *mut YuStorageCompositionCaret,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageCompositionCaret::default() };
    if let Err(status) =
        validate_composition(&session.session, expected_revision, expected_generation)
    {
        return status;
    }
    let affinity = match caret_affinity_from_ffi(affinity) {
        Ok(affinity) => affinity,
        Err(status) => return status,
    };
    let snapshot = session.session.snapshot();
    if snapshot
        .byte_offset_for_utf16(Utf16Offset::new(source_utf16))
        .is_err()
    {
        return YU_STORAGE_INVALID_SELECTION;
    }
    let projection = match composition_projection(&mut session.session) {
        Ok(projection) => projection,
        Err(status) => return status,
    };
    let overlay = match session.session.composition() {
        Some(overlay) => overlay,
        None => return YU_STORAGE_NO_OVERLAY,
    };
    let (active_visual, active_bias) = match composition_active_visual_caret(
        &projection,
        overlay.selection_utf16().start().get(),
        overlay.selection_utf16().end().get(),
    ) {
        Ok(caret) => caret,
        Err(status) => return status,
    };
    let projected = match projected_utf8(&projection) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
    let visual_utf16 = match visual_utf16_offset(&projected, active_visual) {
        Ok(offset) => offset,
        Err(status) => return status,
    };
    let round_trip = match projection.visual_to_source(active_visual, active_bias) {
        Ok(offset) => offset,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let round_trip_source_utf16 = match snapshot.utf16_offset(round_trip) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let (visual_selection_start_utf16, visual_selection_end_utf16) =
        match composition_visual_selection_utf16(&projection, &projected) {
            Ok(selection) => selection,
            Err(status) => return status,
        };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe {
        *output = YuStorageCompositionCaret {
            revision: snapshot.revision().get(),
            generation: session.session.composition_generation(),
            source_utf16,
            visual_utf16,
            round_trip_source_utf16,
            visual_selection_start_utf16,
            visual_selection_end_utf16,
            affinity: match affinity {
                CaretAffinity::Upstream => YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                CaretAffinity::Downstream => YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
            },
        };
    }
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
        let viewport_config = session.session.viewport_config();
        let published = viewport_config.layout();
        if (published.max_width() - max_width).abs() > 0.05
            || (published.line_height() - metrics.line_height()).abs() > 0.05
            || (published.default_advance() - metrics.default_advance()).abs() > 0.05
        {
            return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
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

/// Returns the number of parser-owned blocks in the expected source revision.
/// Block indices are revision-bound and must be queried again after an edit.
///
/// # Safety
/// `session` and `output` must be valid pointers for the duration of this
/// synchronous call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projection_block_count(
    session: *const YuStorageSession,
    expected_revision: u64,
    output: *mut usize,
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
    // SAFETY: `output` was checked above and belongs to the caller.
    unsafe { *output = session.session.block_count() };
    YU_STORAGE_OK
}

/// Returns one parser-owned block projection as owned UTF-8 plus revision,
/// source-range, kind and visual-length metadata. The null/zero-capacity
/// output form is a safe length query and still fills `metadata`.
///
/// # Safety
/// `session` must be a live handle. `metadata` and `written` must be writable;
/// `output` must provide `capacity` writable bytes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projected_block(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    metadata: *mut YuStorageProjectionBlock,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if metadata.is_null() || written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) => index,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let Some((source_range, kind)) = session.session.block_metadata(block_index) else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let projection = match session.session.block_projection(block_index) {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let projected = match projected_utf8(projection.visual()) {
        Ok(projected) => projected,
        Err(status) => return status,
    };
    let snapshot = session.session.snapshot();
    let source_start_utf16 = match snapshot.utf16_offset(source_range.start()) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let source_end_utf16 = match snapshot.utf16_offset(source_range.end()) {
        Ok(offset) => offset.get(),
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let block_index = match u64::try_from(block_index) {
        Ok(index) => index,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let visual_utf8_length = match u64::try_from(projected.len()) {
        Ok(length) => length,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let visual_utf16_length = match u64::try_from(projected.encode_utf16().count()) {
        Ok(length) => length,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    // SAFETY: `metadata` was checked above and belongs to the caller. No
    // metadata is written until every revision/range/projection conversion
    // has succeeded.
    unsafe {
        *metadata = YuStorageProjectionBlock {
            revision: session.session.revision().get(),
            block_index,
            source_start_utf16,
            source_end_utf16,
            visual_utf8_length,
            visual_utf16_length,
            kind,
            projection_kind: projection_kind_tag(&projection),
        };
    }
    write_bytes(projected.as_bytes(), output, capacity, written)
}

/// Returns parser-owned GFM table cell ranges for one projected block.
///
/// The count/fill convention mirrors the other native array queries: callers
/// may pass a null output with zero capacity to learn the required cell count.
/// `row = 0` is the header, `row = 1` is the delimiter row, and body rows start
/// at `row = 2`. Cell text is never copied across the ABI.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable entries when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_projected_table_cells(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    output: *mut YuStorageTableCellRange,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) => index,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let projection = match session.session.block_projection(block_index) {
        Ok(projection) => projection,
        Err(error) => return storage_status(error),
    };
    let encoded = match table_cell_ranges(&session.session.snapshot(), &projection) {
        Ok(encoded) => encoded,
        Err(status) => return status,
    };
    // SAFETY: `written` is checked above and belongs to the caller.
    unsafe { *written = encoded.len() };
    if encoded.is_empty() {
        return YU_STORAGE_OK;
    }
    if capacity == 0 && output.is_null() {
        return YU_STORAGE_OK;
    }
    if capacity < encoded.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller supplied at least `encoded.len()` writable entries.
    unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), output, encoded.len()) };
    YU_STORAGE_OK
}

/// Returns source-backed visible table cell geometry for one parser-owned
/// block. The Markdown delimiter row is intentionally omitted from the
/// returned list, while its source range remains available to the projection
/// and layout layers. The count/fill convention mirrors the other native array
/// queries.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable entries when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_layout_cells(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    output: *mut YuStorageTableLayoutCell,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` was checked for null and belongs to the caller.
    unsafe { *written = 0 };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) if index < session.session.block_count() => index,
        _ => return YU_STORAGE_INVALID_SELECTION,
    };
    let config = LayoutConfig::new(max_width, line_height).with_default_advance(default_advance);
    let layout = match session.session.block_layout(block_index, config) {
        Ok(layout) => layout,
        Err(error) => return storage_status(error),
    };
    let Some(table) = layout.table() else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let snapshot = session.session.snapshot();
    let block_index = match u64::try_from(block_index) {
        Ok(block_index) => block_index,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let encoded = match table_layout_cells_metadata(
        &snapshot,
        session.session.revision().get(),
        block_index,
        table,
    ) {
        Ok(encoded) => encoded,
        Err(status) => return status,
    };
    // SAFETY: `written` was checked above and belongs to the caller.
    unsafe { *written = encoded.len() };
    if encoded.is_empty() {
        return YU_STORAGE_OK;
    }
    if capacity == 0 && output.is_null() {
        return YU_STORAGE_OK;
    }
    if capacity < encoded.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller supplied at least `encoded.len()` writable entries.
    unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), output, encoded.len()) };
    YU_STORAGE_OK
}

/// Returns visible table cell geometry with a session-only column override.
/// The override is applied to an owned transient layout snapshot for this
/// call; it never changes Markdown source, selection, history or the
/// editor-owned layout cache. Only `YU_STORAGE_TABLE_RESIZE_COLUMN` is
/// supported in this first geometry bridge; row-height persistence remains a
/// later variable-row layout concern.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable entries when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_layout_cells_with_resize(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    resize_kind: u8,
    resize_index: u64,
    resize_delta: f32,
    output: *mut YuStorageTableLayoutCell,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` was checked above and belongs to the caller.
    unsafe { *written = 0 };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    if resize_kind != YU_STORAGE_TABLE_RESIZE_COLUMN {
        return YU_STORAGE_INVALID_SELECTION;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) if index < session.session.block_count() => index,
        _ => return YU_STORAGE_INVALID_SELECTION,
    };
    let resize_index = match usize::try_from(resize_index) {
        Ok(index) => index,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let config = LayoutConfig::new(max_width, line_height).with_default_advance(default_advance);
    let mut layout = match session.session.block_layout(block_index, config) {
        Ok(layout) => layout,
        Err(error) => return storage_status(error),
    };
    if layout
        .apply_table_column_resize(resize_index, resize_delta)
        .is_err()
    {
        return YU_STORAGE_INVALID_SELECTION;
    }
    let Some(table) = layout.table() else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let snapshot = session.session.snapshot();
    let block_index = match u64::try_from(block_index) {
        Ok(block_index) => block_index,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let encoded = match table_layout_cells_metadata(
        &snapshot,
        session.session.revision().get(),
        block_index,
        table,
    ) {
        Ok(encoded) => encoded,
        Err(status) => return status,
    };
    // SAFETY: `written` was checked above and belongs to the caller.
    unsafe { *written = encoded.len() };
    if encoded.is_empty() {
        return YU_STORAGE_OK;
    }
    if capacity == 0 && output.is_null() {
        return YU_STORAGE_OK;
    }
    if capacity < encoded.len() {
        return YU_STORAGE_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller supplied at least `encoded.len()` writable entries.
    unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), output, encoded.len()) };
    YU_STORAGE_OK
}

/// Resolves a local table point to the visible source-backed cell containing
/// it. Points outside the table return `YU_STORAGE_INVALID_SELECTION`.
///
/// # Safety
/// `session` and `output` must be live/writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_cell_hit_test(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    point_x: f32,
    point_y: f32,
    output: *mut YuStorageTableCellHit,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `output` was checked for null and belongs to the caller.
    unsafe { *output = YuStorageTableCellHit::default() };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) if index < session.session.block_count() => index,
        _ => return YU_STORAGE_INVALID_SELECTION,
    };
    let config = LayoutConfig::new(max_width, line_height).with_default_advance(default_advance);
    let layout = match session.session.block_layout(block_index, config) {
        Ok(layout) => layout,
        Err(error) => return storage_status(error),
    };
    let Some(table) = layout.table() else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let hit = match table.hit_test(LayoutPoint::new(point_x, point_y)) {
        Ok(Some(hit)) => hit,
        Ok(None) => return YU_STORAGE_INVALID_SELECTION,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let snapshot = session.session.snapshot();
    let metadata = match table_layout_hit_metadata(
        &snapshot,
        session.session.revision().get(),
        match u64::try_from(block_index) {
            Ok(block_index) => block_index,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        },
        hit,
    ) {
        Ok(metadata) => metadata,
        Err(status) => return status,
    };
    // SAFETY: `output` was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Resolves a local table point to an internal column or row divider. Outer
/// table edges are not resize targets. The result is Revision-bound and does
/// not mutate source, selection or layout state.
///
/// # Safety
/// `session` and `output` must be live/writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_hit_test(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    point_x: f32,
    point_y: f32,
    tolerance: f32,
    output: *mut YuStorageTableResizeHit,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `output` was checked for null and belongs to the caller.
    unsafe { *output = YuStorageTableResizeHit::default() };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) if index < session.session.block_count() => index,
        _ => return YU_STORAGE_INVALID_SELECTION,
    };
    let config = LayoutConfig::new(max_width, line_height).with_default_advance(default_advance);
    let layout = match session.session.block_layout(block_index, config) {
        Ok(layout) => layout,
        Err(error) => return storage_status(error),
    };
    let Some(table) = layout.table() else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let hit = match table.resize_hit_test(LayoutPoint::new(point_x, point_y), tolerance) {
        Ok(Some(hit)) => hit,
        Ok(None) => return YU_STORAGE_INVALID_SELECTION,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let block_index = match u64::try_from(block_index) {
        Ok(block_index) => block_index,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let metadata =
        match table_resize_hit_metadata(session.session.revision().get(), block_index, hit) {
            Ok(metadata) => metadata,
            Err(status) => return status,
        };
    // SAFETY: `output` was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Starts one Revision-bound native table resize gesture. The hit result is
/// copied to `output`, while the actual gesture remains Rust-owned on the
/// session until update, finish or cancel. The initial preview is transient
/// and does not mutate Markdown source or the canonical layout cache.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_begin(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
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
    if let Err(status) = validate_table_resize_revision(session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) if index < session.session.block_count() => index,
        _ => return YU_STORAGE_INVALID_SELECTION,
    };
    let config = LayoutConfig::new(max_width, line_height).with_default_advance(default_advance);
    let layout = match session.session.block_layout(block_index, config) {
        Ok(layout) => layout,
        Err(error) => return storage_status(error),
    };
    let Some(table) = layout.table() else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let hit = match table.resize_hit_test(LayoutPoint::new(point_x, point_y), tolerance) {
        Ok(Some(hit)) => hit,
        Ok(None) | Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let metadata = match begin_table_resize_session(session, block_index, hit, pointer_position) {
        Ok(metadata) => metadata,
        Err(status) => return status,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
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
    let configured = session.session.viewport_config().layout();
    if (configured.max_width() - max_width).abs() > 0.05
        || (configured.line_height() - metrics.line_height()).abs() > 0.05
        || (configured.default_advance() - metrics.default_advance()).abs() > 0.05
    {
        return Err(YU_STORAGE_INVALID_VIEWPORT_CONFIG);
    }

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

/// Resolves a document-space point through the CoreText-shaped viewport and
/// returns an internal table divider without mutating the session.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_table_resize_hit_test(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    point_x: f32,
    point_y: f32,
    tolerance: f32,
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
    let block_index = match u64::try_from(block_index) {
        Ok(value) => value,
        Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let metadata =
        match table_resize_hit_metadata(session.session.revision().get(), block_index, hit) {
            Ok(value) => value,
            Err(status) => return status,
        };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Starts a CoreText-shaped table resize gesture from a document-space point.
/// The Rust session owns the gesture after this call; the native shell only
/// needs to forward pointer movement along the returned divider axis.
///
/// # Safety
/// `session` must be a live handle and `output` must be writable.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_table_resize_begin_at_point(
    session: *mut YuStorageSession,
    expected_revision: u64,
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
    if !pointer_position.is_finite() {
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
    let metadata = match begin_table_resize_session(session, block_index, hit, pointer_position) {
        Ok(value) => value,
        Err(status) => return status,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// macOS/CoreText-shaped variant of table resize begin. The hit-test layout
/// uses the same system shaper and font size as the retained render-host
/// frame, so the divider captured by the gesture is in the same geometry
/// space that scene/render-plan assembly will later consume.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_table_resize_begin(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
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
    if let Err(status) = validate_table_resize_revision(session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) if index < session.session.block_count() => index,
        _ => return YU_STORAGE_INVALID_SELECTION,
    };
    let (shaper, _metrics, config) = match core_text_system_ui_layout(size, max_width) {
        Ok(value) => value,
        Err(status) => return status,
    };
    let layout = {
        let document = session.session.document_mut().editor_mut();
        match document.block_layout_with_shaper(block_index, config, &shaper) {
            Ok(layout) => layout.clone(),
            Err(error) => return status_from_editor_error(error),
        }
    };
    let Some(table) = layout.table() else {
        return YU_STORAGE_INVALID_SELECTION;
    };
    let hit = match table.resize_hit_test(LayoutPoint::new(point_x, point_y), tolerance) {
        Ok(Some(hit)) => hit,
        Ok(None) | Err(_) => return YU_STORAGE_INVALID_SELECTION,
    };
    let metadata = match begin_table_resize_session(session, block_index, hit, pointer_position) {
        Ok(metadata) => metadata,
        Err(status) => return status,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Updates the Rust-owned table resize gesture and returns the transient
/// geometry that the next render-host frame will consume.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_update(
    session: *mut YuStorageSession,
    expected_revision: u64,
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
    if let Err(status) = validate_table_resize_revision(session, expected_revision) {
        return status;
    }
    let revision = session.session.revision();
    let Some(gesture) = session.table_resize_gesture.as_mut() else {
        return YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE;
    };
    if let Err(error) = gesture.update(revision, pointer_position) {
        session.table_resize_gesture = None;
        session.table_resize_override = None;
        return table_resize_gesture_status(error);
    }
    let commit = gesture.preview();
    let metadata = match table_resize_commit_metadata(commit) {
        Ok(metadata) => metadata,
        Err(status) => return status,
    };
    session.table_resize_override = Some(commit);
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Finishes the current table resize gesture and keeps its final geometry as
/// a session-only override for subsequent retained frames. No Markdown
/// transaction is created by this function.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_finish(
    session: *mut YuStorageSession,
    expected_revision: u64,
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
    if let Err(status) = validate_table_resize_revision(session, expected_revision) {
        return status;
    }
    let revision = session.session.revision();
    let Some(gesture) = session.table_resize_gesture.take() else {
        return YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE;
    };
    let commit = match gesture.finish(revision) {
        Ok(commit) => commit,
        Err(error) => {
            session.table_resize_override = None;
            return table_resize_gesture_status(error);
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

/// Cancels a current table resize gesture and removes any session-only table
/// geometry override. It is also safe to call after finish to clear the final
/// preview; in that case the function returns `YU_STORAGE_OK`.
///
/// # Safety
/// `session` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_table_resize_cancel(
    session: *mut YuStorageSession,
    expected_revision: u64,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_table_resize_revision(session, expected_revision) {
        return status;
    }
    let Some(gesture) = session.table_resize_gesture.take() else {
        session.table_resize_override = None;
        return YU_STORAGE_OK;
    };
    session.table_resize_override = None;
    gesture
        .cancel(session.session.revision())
        .map_or_else(table_resize_gesture_status, |_| YU_STORAGE_OK)
}

/// Returns revision-bound metrics layout metadata for one parser-owned block.
/// The block remains owned by the Rust editor; only source ranges, visual
/// length and measured scalar geometry cross the ABI.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_block_layout(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    output: *mut YuStorageBlockLayout,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageBlockLayout::default() };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let block_index = match usize::try_from(block_index) {
        Ok(index) if index < session.session.block_count() => index,
        _ => return YU_STORAGE_INVALID_SELECTION,
    };
    let config = LayoutConfig::new(max_width, line_height).with_default_advance(default_advance);
    let layout = match session.session.block_layout(block_index, config) {
        Ok(layout) => layout,
        Err(error) => return storage_status(error),
    };
    let metadata = match block_layout_metadata(
        &mut session.session,
        block_index,
        &layout,
        line_height,
        default_advance,
        0,
    ) {
        Ok(metadata) => metadata,
        Err(status) => return status,
    };
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = metadata };
    YU_STORAGE_OK
}

/// Returns revision-bound CoreText metrics without requiring a parser-owned
/// block. Native hosts use this to configure the viewport for an empty
/// document before requesting a render-host frame.
///
/// # Safety
/// `session` must be null or a live handle and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_font_metrics(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    output: *mut YuStorageMacosFontMetrics,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageMacosFontMetrics::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session, expected_revision, size, max_width);
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        let (_, metrics, _) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe {
            *output = YuStorageMacosFontMetrics {
                revision: session.session.revision().get(),
                size,
                line_height: metrics.line_height(),
                default_advance: metrics.default_advance(),
            };
        }
        YU_STORAGE_OK
    }
}

/// Returns one block's layout using the macOS System UI CoreText shaper. On
/// non-macOS targets the stable symbol returns `CORE_TEXT_UNAVAILABLE` after
/// clearing output.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_block_layout(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
    size: f32,
    max_width: f32,
    output: *mut YuStorageBlockLayout,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if output.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output was checked for null and belongs to the caller.
    unsafe { *output = YuStorageBlockLayout::default() };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session, expected_revision, block_index, size, max_width);
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(status) = validate_revision(&session.session, expected_revision) {
            return status;
        }
        let block_index = match usize::try_from(block_index) {
            Ok(index) if index < session.session.block_count() => index,
            _ => return YU_STORAGE_INVALID_SELECTION,
        };
        let (shaper, metrics, config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        let layout = match session
            .session
            .block_layout_with_shaper(block_index, config, &shaper)
        {
            Ok(layout) => layout,
            Err(error) => return storage_status(error),
        };
        let metadata = match block_layout_metadata(
            &mut session.session,
            block_index,
            &layout,
            metrics.line_height(),
            metrics.default_advance(),
            1,
        ) {
            Ok(metadata) => metadata,
            Err(status) => return status,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe { *output = metadata };
        YU_STORAGE_OK
    }
}

/// Resolves a source caret through one block's CoreText-shaped layout. The
/// result is block-local and source-backed; no CoreText object crosses the
/// boundary. Non-macOS targets return `CORE_TEXT_UNAVAILABLE`.
///
/// # Safety
/// `session` must be live and `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_block_caret(
    session: *mut YuStorageSession,
    expected_revision: u64,
    block_index: u64,
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
            block_index,
            source_utf16,
            affinity,
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
        let affinity = match caret_affinity_from_ffi(affinity) {
            Ok(affinity) => affinity,
            Err(status) => return status,
        };
        let block_index = match usize::try_from(block_index) {
            Ok(index) if index < session.session.block_count() => index,
            _ => return YU_STORAGE_INVALID_SELECTION,
        };
        let (shaper, metrics, config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        let snapshot = session.session.snapshot();
        if snapshot
            .byte_offset_for_utf16(Utf16Offset::new(source_utf16))
            .is_err()
        {
            return YU_STORAGE_INVALID_SELECTION;
        }
        let layout = match session
            .session
            .block_layout_with_shaper(block_index, config, &shaper)
        {
            Ok(layout) => layout,
            Err(error) => return storage_status(error),
        };
        let caret = match block_caret_from_layout(
            &session.session,
            block_index,
            source_utf16,
            affinity,
            &layout,
            metrics.line_height(),
            1,
        ) {
            Ok(caret) => caret,
            Err(status) => return status,
        };
        // SAFETY: output was checked for null and belongs to the caller.
        unsafe { *output = caret };
        YU_STORAGE_OK
    }
}

/// Applies native viewport metrics to the revision-bound Rust viewport policy.
/// This changes only layout estimates and measured-block state; it never
/// changes the canonical source or its revision.
///
/// # Safety
/// `session` must be null or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_set_viewport_config(
    session: *mut YuStorageSession,
    expected_revision: u64,
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    estimated_block_height: f32,
    overscan: f32,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if let Err(status) = validate_revision(&session.session, expected_revision) {
        return status;
    }
    let config = ViewportConfig::new(
        LayoutConfig::new(max_width, line_height).with_default_advance(default_advance),
        estimated_block_height,
        overscan,
    );
    session
        .session
        .set_viewport_config(config)
        .map_or(YU_STORAGE_INVALID_VIEWPORT_CONFIG, |_| YU_STORAGE_OK)
}

/// Returns the current macOS CoreText-shaped viewport block metadata using a
/// count/fill ABI. The first call may pass `capacity = 0` and a null `blocks`
/// pointer to learn the required count. The snapshot header is written on
/// both calls; block output is never partially written when capacity is too
/// small. Non-macOS targets return `CORE_TEXT_UNAVAILABLE` after clearing
/// output.
///
/// # Safety
/// `session` must be null or a live handle. `snapshot` and `written` must
/// point to writable values. `blocks` must point to `capacity` writable values
/// when `capacity > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_shaped_viewport_blocks(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    snapshot: *mut YuStorageShapedViewportSnapshot,
    blocks: *mut YuStorageShapedViewportBlock,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if snapshot.is_null() || written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && blocks.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output pointers were checked for null and belong to the caller.
    unsafe {
        *snapshot = YuStorageShapedViewportSnapshot::default();
        *written = 0;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            blocks,
            capacity,
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
        let viewport_config = session.session.viewport_config();
        let layout_config = viewport_config.layout();
        if (layout_config.max_width() - max_width).abs() > 0.05
            || (layout_config.line_height() - metrics.line_height()).abs() > 0.05
            || (layout_config.default_advance() - metrics.default_advance()).abs() > 0.05
        {
            return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
        }
        let viewport = ViewportRect::new(scroll_y, viewport_height);
        let viewport_snapshot = {
            let document = session.session.document_mut().editor_mut();
            match if document.composition().is_some() {
                document.visible_blocks_with_composition_and_shaper(viewport, &shaper)
            } else {
                document.visible_blocks_with_shaper(viewport, &shaper)
            } {
                Ok(snapshot) => snapshot,
                Err(error) => return status_from_editor_error(error),
            }
        };
        let source = session.session.snapshot();
        let mut encoded = Vec::with_capacity(viewport_snapshot.blocks().len());
        for block in viewport_snapshot.blocks() {
            let source_start_utf16 = match source.utf16_offset(block.source().start()) {
                Ok(offset) => offset.get(),
                Err(_) => return YU_STORAGE_INVALID_SELECTION,
            };
            let source_end_utf16 = match source.utf16_offset(block.source().end()) {
                Ok(offset) => offset.get(),
                Err(_) => return YU_STORAGE_INVALID_SELECTION,
            };
            let block_index = match u64::try_from(block.index()) {
                Ok(index) => index,
                Err(_) => return YU_STORAGE_INVALID_SELECTION,
            };
            encoded.push(YuStorageShapedViewportBlock {
                revision: viewport_snapshot.revision().get(),
                block_index,
                source_start_utf16,
                source_end_utf16,
                y: block.y(),
                height: block.height(),
                measured: u8::from(block.is_measured()),
                kind: viewport_block_kind(block.kind()),
            });
        }
        let block_start = match u64::try_from(viewport_snapshot.range().start()) {
            Ok(index) => index,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let block_end = match u64::try_from(viewport_snapshot.range().end()) {
            Ok(index) => index,
            Err(_) => return YU_STORAGE_INVALID_SELECTION,
        };
        let header = YuStorageShapedViewportSnapshot {
            revision: viewport_snapshot.revision().get(),
            block_start,
            block_end,
            content_height: viewport_snapshot.content_height(),
            scroll_y,
            viewport_height,
            max_scroll_y: (viewport_snapshot.content_height() - viewport_height).max(0.0),
        };
        // SAFETY: pointers were checked above and belong to the caller.
        unsafe {
            *snapshot = header;
            *written = encoded.len();
        }
        if capacity == 0 && blocks.is_null() {
            return YU_STORAGE_OK;
        }
        if encoded.len() > capacity {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded.is_empty() {
            // SAFETY: capacity was checked against the encoded block count.
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), blocks, encoded.len()) };
        }
        YU_STORAGE_OK
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
        let (shaper, metrics, _layout_config) = match core_text_system_ui_layout(size, max_width) {
            Ok(layout) => layout,
            Err(status) => return status,
        };
        let layout_config = session.session.viewport_config().layout();
        if (layout_config.max_width() - max_width).abs() > 0.05
            || (layout_config.line_height() - metrics.line_height()).abs() > 0.05
            || (layout_config.default_advance() - metrics.default_advance()).abs() > 0.05
        {
            return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
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

#[cfg(target_os = "macos")]
type MacosVisualDecorations = (
    YuStorageMacosVisualDecorationSnapshot,
    YuStorageMacosVisualDecorationCaret,
    Vec<YuStorageMacosVisualDecorationRect>,
);

/// Builds the Rust/CoreText-shaped selection and caret geometry used by the
/// native decoration sibling. The returned rectangles are document-space;
/// the caller owns the final scroll-to-viewport transform. Active marked text
/// uses the same uncached composition projection and viewport layout as the
/// Rust surface, so its transient caret/selection remains generation-bound and
/// does not require a second TextKit paint path.
#[cfg(target_os = "macos")]
fn macos_visual_decorations(
    session: &mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
) -> Result<MacosVisualDecorations, i32> {
    validate_revision(&session.session, expected_revision)?;
    if session.session.composition_generation() != expected_generation {
        return Err(YU_STORAGE_STALE_COMPOSITION);
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
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    let (shaper, metrics, _layout_config) = core_text_system_ui_layout(size, max_width)?;
    let viewport_config = session.session.viewport_config();
    let configured = viewport_config.layout();
    if (configured.max_width() - max_width).abs() > 0.05
        || (configured.line_height() - metrics.line_height()).abs() > 0.05
        || (configured.default_advance() - metrics.default_advance()).abs() > 0.05
    {
        return Err(YU_STORAGE_INVALID_VIEWPORT_CONFIG);
    }

    let viewport = ViewportRect::new(scroll_y, viewport_height);
    let composition = session.session.composition().map(|overlay| {
        (
            overlay.replacement_range().start(),
            overlay.selection_utf16().start().get(),
            overlay.selection_utf16().end().get(),
        )
    });
    let composition_blocks = session
        .session
        .document()
        .editor()
        .composition_block_range();
    let (selection, viewport_snapshot) = {
        let document = session.session.document_mut().editor_mut();
        let selection = document.selection();
        let viewport_snapshot = if composition.is_some() {
            document
                .visible_blocks_with_composition_and_shaper(viewport, &shaper)
                .map_err(status_from_editor_error)?
        } else {
            document
                .visible_blocks_with_shaper(viewport, &shaper)
                .map_err(status_from_editor_error)?
        };
        (selection, viewport_snapshot)
    };
    if viewport_snapshot.revision().get() != expected_revision {
        return Err(YU_STORAGE_STALE_REVISION);
    }

    let mut caret = YuStorageMacosVisualDecorationCaret {
        revision: expected_revision,
        ..YuStorageMacosVisualDecorationCaret::default()
    };
    let focus_block = if let Some((replacement_start, _, _)) = composition {
        composition_blocks
            .as_ref()
            .map(|span| span.start)
            .or_else(|| {
                session
                    .session
                    .document()
                    .editor()
                    .block_index_for_source(replacement_start)
            })
    } else {
        session
            .session
            .document()
            .editor()
            .block_index_for_source(selection.focus())
    };
    let mut rectangles = Vec::new();
    let selection_range = selection.ordered_range();
    let document = session.session.document_mut().editor_mut();

    for block in viewport_snapshot.blocks().iter().copied() {
        let block_index = block.index();
        let block_index_u64 =
            u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
        let layout = if composition_blocks
            .as_ref()
            .is_some_and(|span| span.contains(&block_index))
        {
            document
                .block_layout_with_composition_and_shaper(block_index, configured, &shaper)
                .map_err(status_from_editor_error)?
        } else {
            document
                .block_layout_with_shaper(block_index, configured, &shaper)
                .map_err(status_from_editor_error)?
                .clone()
        };
        if layout.revision().get() != expected_revision {
            return Err(YU_STORAGE_STALE_REVISION);
        }

        if focus_block == Some(block_index) {
            let (layout_caret, affinity) = if let Some((_, start_utf16, end_utf16)) = composition {
                let (visual, bias) =
                    composition_active_visual_caret(layout.projection(), start_utf16, end_utf16)?;
                (
                    layout
                        .caret_for_visual(visual, bias)
                        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            } else {
                let bias = projection_bias_from_affinity(selection.affinity());
                (
                    layout
                        .caret_for_source(selection.focus(), bias)
                        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
                    affinity_to_ffi(selection.affinity()),
                )
            };
            let point = layout_caret.point();
            if !point.x().is_finite() || !point.y().is_finite() {
                return Err(YU_STORAGE_EDITOR_ERROR);
            }
            caret = YuStorageMacosVisualDecorationCaret {
                revision: expected_revision,
                block_index: block_index_u64,
                line_index: u64::try_from(layout_caret.line())
                    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
                x: point.x(),
                y: block.y() + point.y(),
                width: 1.0,
                height: metrics.line_height(),
                affinity,
                present: 1,
            };
            if !caret.y.is_finite() {
                return Err(YU_STORAGE_EDITOR_ERROR);
            }
        }

        let (visual_start, visual_end) = if composition.is_some() {
            let Some(visual_selection) = layout.projection().composition_selection_visual() else {
                continue;
            };
            (visual_selection.start(), visual_selection.end())
        } else {
            if selection.is_empty() {
                continue;
            }
            let block_source = block.source();
            let start = selection_range.start().max(block_source.start());
            let end = selection_range.end().min(block_source.end());
            if start >= end {
                continue;
            }
            (
                layout
                    .projection()
                    .source_to_visual(start, ProjectionBias::Before)
                    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
                layout
                    .projection()
                    .source_to_visual(end, ProjectionBias::After)
                    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
            )
        };
        if visual_start >= visual_end {
            continue;
        }
        for line in layout.lines() {
            let line_visual = line.visual();
            let line_start = line_visual.start().max(visual_start);
            let line_end = line_visual.end().min(visual_end);
            if line_start >= line_end {
                continue;
            }
            let mut left = f32::INFINITY;
            let mut right = f32::NEG_INFINITY;
            for cluster_index in line.cluster_range() {
                let cluster = layout.clusters()[cluster_index];
                let cluster_visual = cluster.visual();
                if cluster.is_line_break()
                    || cluster_visual.end() <= line_start
                    || cluster_visual.start() >= line_end
                {
                    continue;
                }
                let x = cluster.x();
                let cluster_right = x + cluster.width();
                if !x.is_finite() || !cluster_right.is_finite() || cluster.width() <= 0.0 {
                    continue;
                }
                left = left.min(x);
                right = right.max(cluster_right);
            }
            if !left.is_finite() || !right.is_finite() || right <= left {
                continue;
            }
            let y = block.y() + line.y();
            if !y.is_finite() {
                return Err(YU_STORAGE_EDITOR_ERROR);
            }
            rectangles.push(YuStorageMacosVisualDecorationRect {
                revision: expected_revision,
                block_index: block_index_u64,
                line_index: u64::try_from(line.index())
                    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
                x: left,
                y,
                width: right - left,
                height: metrics.line_height(),
                kind: 0,
            });
        }
    }

    let selection_count =
        u64::try_from(rectangles.len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let content_height = viewport_snapshot.content_height();
    if !content_height.is_finite() || !metrics.line_height().is_finite() {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    let snapshot = YuStorageMacosVisualDecorationSnapshot {
        revision: expected_revision,
        composition_generation: session.session.composition_generation(),
        selection_count,
        caret_present: caret.present,
        content_height,
        scroll_y,
        viewport_height,
        max_scroll_y: (content_height - viewport_height).max(0.0),
        viewport_width: max_width,
    };
    Ok((snapshot, caret, rectangles))
}

/// Returns Rust/CoreText-shaped visual selection rectangles and caret geometry
/// using a revision- and composition-generation-bound count/fill ABI. The
/// first call may pass `capacity = 0` and a null `rects` pointer. No partial
/// rectangle writes occur when the caller's capacity is too small.
///
/// # Safety
/// `session` must be live. `snapshot`, `caret`, and `written` must be writable;
/// `rects` must point to `capacity` writable values when `capacity > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_visual_decorations(
    session: *mut YuStorageSession,
    expected_revision: u64,
    expected_generation: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    snapshot: *mut YuStorageMacosVisualDecorationSnapshot,
    caret: *mut YuStorageMacosVisualDecorationCaret,
    rects: *mut YuStorageMacosVisualDecorationRect,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if snapshot.is_null() || caret.is_null() || written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && rects.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output pointers were checked for null and belong to the caller.
    unsafe {
        *snapshot = YuStorageMacosVisualDecorationSnapshot::default();
        *caret = YuStorageMacosVisualDecorationCaret::default();
        *written = 0;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            expected_generation,
            size,
            max_width,
            scroll_y,
            viewport_height,
            rects,
            capacity,
        );
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        let (header, decoration_caret, encoded) = match macos_visual_decorations(
            session,
            expected_revision,
            expected_generation,
            size,
            max_width,
            scroll_y,
            viewport_height,
        ) {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: output pointers were checked above.
        unsafe {
            *snapshot = header;
            *caret = decoration_caret;
            *written = encoded.len();
        }
        if capacity == 0 && rects.is_null() {
            return YU_STORAGE_OK;
        }
        if encoded.len() > capacity {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded.is_empty() {
            // SAFETY: capacity was checked against the encoded rectangle count.
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), rects, encoded.len()) };
        }
        YU_STORAGE_OK
    }
}

#[cfg(target_os = "macos")]
fn macos_visual_scene(
    session: &mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
) -> Result<
    (
        YuStorageVisualSceneSnapshot,
        Vec<YuStorageVisualScenePrimitive>,
    ),
    i32,
> {
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
    let (shaper, metrics, _layout_config) = core_text_system_ui_layout(size, max_width)?;
    let viewport_config = session.session.viewport_config();
    let layout_config = viewport_config.layout();
    if (layout_config.max_width() - max_width).abs() > 0.05
        || (layout_config.line_height() - metrics.line_height()).abs() > 0.05
        || (layout_config.default_advance() - metrics.default_advance()).abs() > 0.05
    {
        return Err(YU_STORAGE_INVALID_VIEWPORT_CONFIG);
    }

    let viewport = ViewportRect::new(scroll_y, viewport_height);
    let viewport_snapshot = {
        let document = session.session.document_mut().editor_mut();
        if document.composition().is_some() {
            document
                .visible_blocks_with_composition_and_shaper(viewport, &shaper)
                .map_err(status_from_editor_error)?
        } else {
            document
                .visible_blocks_with_shaper(viewport, &shaper)
                .map_err(status_from_editor_error)?
        }
    };
    let revision = viewport_snapshot.revision();
    let source = session.session.snapshot();
    let geometries = viewport_snapshot
        .blocks()
        .iter()
        .copied()
        .map(|block| {
            ViewportBlockGeometry::new(
                revision,
                block.index(),
                block.source(),
                block.y(),
                block.height(),
                block.is_measured(),
                viewport_block_kind(block.kind()),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let input = ViewportSceneInput::new(
        revision,
        viewport_snapshot.range().start()..viewport_snapshot.range().end(),
        viewport_snapshot.content_height(),
        geometries,
    )
    .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let scene_height = viewport_snapshot
        .content_height()
        .max(viewport_height)
        .max(1.0);
    let scene_viewport =
        Rect::new(0.0, 0.0, max_width, scene_height).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let mut builder =
        SceneBuilder::new(revision, scene_viewport).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let mut metadata = Vec::with_capacity(input.blocks().len().saturating_mul(2));

    for geometry in input.blocks().iter().copied() {
        let source_start_utf16 = source
            .utf16_offset(geometry.source().start())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get();
        let source_end_utf16 = source
            .utf16_offset(geometry.source().end())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get();

        let background = Rect::new(0.0, geometry.y(), max_width, geometry.height())
            .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
        builder
            .fill_rect(background, Rgba8::new(246, 247, 249, 255))
            .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
        metadata.push((
            YU_STORAGE_SCENE_PRIMITIVE_BACKGROUND,
            geometry.index(),
            source_start_utf16,
            source_end_utf16,
        ));

        let source_len = source_end_utf16.saturating_sub(source_start_utf16);
        let source_len = u32::try_from(source_len.min(u64::from(u32::MAX))).unwrap_or(u32::MAX);
        let text_width = if source_len == 0 {
            0.0
        } else {
            (source_len as f32 * metrics.default_advance())
                .min(max_width)
                .max(metrics.default_advance().min(max_width))
        };
        let text_height = geometry.height().min(metrics.line_height());
        let text_y = geometry.y() + (geometry.height() - text_height) * 0.5;
        let text_bounds =
            Rect::new(0.0, text_y, text_width, text_height).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
        builder
            .fill_rect(text_bounds, Rgba8::new(32, 35, 40, 255))
            .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
        metadata.push((
            YU_STORAGE_SCENE_PRIMITIVE_TEXT_BOUNDS,
            geometry.index(),
            source_start_utf16,
            source_end_utf16,
        ));
    }

    let scene = builder.finish();
    if scene.primitives().len() != metadata.len() {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }
    let mut primitives = Vec::with_capacity(scene.primitives().len());
    for (primitive, (kind, block_index, source_start_utf16, source_end_utf16)) in
        scene.primitives().iter().copied().zip(metadata)
    {
        let bounds = primitive.bounds();
        primitives.push(YuStorageVisualScenePrimitive {
            revision: scene.revision().get(),
            block_index: u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
            source_start_utf16,
            source_end_utf16,
            x: bounds.x(),
            y: bounds.y(),
            width: bounds.width(),
            height: bounds.height(),
            kind,
        });
    }
    let block_start = u64::try_from(viewport_snapshot.range().start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let block_end =
        u64::try_from(viewport_snapshot.range().end()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let primitive_count =
        u64::try_from(primitives.len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let snapshot = YuStorageVisualSceneSnapshot {
        revision: scene.revision().get(),
        block_start,
        block_end,
        primitive_count,
        content_height: viewport_snapshot.content_height(),
        scroll_y,
        viewport_height,
        max_scroll_y: (viewport_snapshot.content_height() - viewport_height).max(0.0),
        viewport_width: max_width,
    };
    Ok((snapshot, primitives))
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
fn renderable_image_count(
    source: &TextSnapshot,
    layout: &LayoutSnapshot,
    definitions: &yu_markdown::ReferenceDefinitionIndex,
) -> usize {
    layout
        .images()
        .iter()
        .filter(|placement| {
            layout
                .projection()
                .images()
                .iter()
                .copied()
                .find(|image| image.source() == placement.source())
                .and_then(|image| image_resource_key(source, image, definitions))
                .is_some()
        })
        .count()
}

#[cfg(target_os = "macos")]
fn macos_visual_images(
    session: &mut YuStorageSession,
    expected_revision: u64,
) -> Result<Vec<YuStorageVisualImage>, i32> {
    validate_revision(&session.session, expected_revision)?;
    let source = session.session.snapshot();
    let definitions = session
        .session
        .document()
        .editor()
        .markdown()
        .reference_definitions()
        .clone();
    let host = session.macos_render_host.as_ref();
    let mut encoded = Vec::new();
    for block_index in 0..session.session.block_count() {
        let projection = session
            .session
            .block_projection(block_index)
            .map_err(storage_status)?;
        for image in projection.visual().images().iter().copied() {
            let destination = image_destination(&source, image, &definitions);
            let (source_start_utf16, source_end_utf16) =
                image_utf16_range(&source, Some(image.source()))?;
            let (label_start_utf16, label_end_utf16) =
                image_utf16_range(&source, Some(image.label()))?;
            let (destination_start_utf16, destination_end_utf16) =
                image_utf16_range(&source, destination)?;
            let (reference_start_utf16, reference_end_utf16) =
                image_utf16_range(&source, image.reference())?;
            let resource_key = image_resource_key(&source, image, &definitions);
            let resource_fingerprint = resource_key
                .as_ref()
                .map(|key| key.fingerprint())
                .unwrap_or(0);
            let resource_status =
                macos_image_resource_status(host, resource_key.as_ref(), source.revision().get());
            encoded.push(YuStorageVisualImage {
                revision: source.revision().get(),
                block_index: u64::try_from(block_index)
                    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
                source_start_utf16,
                source_end_utf16,
                label_start_utf16,
                label_end_utf16,
                destination_start_utf16,
                destination_end_utf16,
                reference_start_utf16,
                reference_end_utf16,
                resource_fingerprint,
                kind: if image.is_reference() {
                    YU_STORAGE_IMAGE_REFERENCE
                } else {
                    YU_STORAGE_IMAGE_INLINE
                },
                resource_status,
            });
        }
    }
    Ok(encoded)
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
fn macos_visual_embedded_resources(
    session: &mut YuStorageSession,
    expected_revision: u64,
) -> Result<Vec<YuStorageVisualEmbeddedResource>, i32> {
    validate_revision(&session.session, expected_revision)?;
    let source = session.session.snapshot();
    let revision = source.revision();
    let mut encoded = Vec::new();
    for block_index in 0..session.session.block_count() {
        let projection = session
            .session
            .block_projection(block_index)
            .map_err(storage_status)?;
        let BlockProjection::FencedCode(code) = projection else {
            continue;
        };
        let Some(kind) = embedded_resource_kind(&source, code.info_string()) else {
            continue;
        };
        let embedded_kind = embedded_resource_kind_from_ffi(kind).ok_or(YU_STORAGE_EDITOR_ERROR)?;
        let content = embedded_resource_content(&source, code.content())?;
        // An empty fenced body is still a valid source-backed resource. Keep
        // it in the cache path with a trim-neutral sentinel so the FFI can
        // publish unsupported/failed state instead of rejecting metadata.
        let content = if content.is_empty() {
            "\n".to_owned()
        } else {
            content
        };
        let request =
            EmbeddedRenderRequest::new(revision, code.source_range(), embedded_kind, content)
                .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
        let resource_status = session
            .macos_embedded_resources
            .status_for(request, revision)?;
        let (source_start_utf16, source_end_utf16) =
            image_utf16_range(&source, Some(code.source_range()))?;
        let (info_start_utf16, info_end_utf16) =
            image_utf16_range(&source, Some(code.info_string()))?;
        let (content_start_utf16, content_end_utf16) =
            image_utf16_range(&source, Some(code.content()))?;
        encoded.push(YuStorageVisualEmbeddedResource {
            revision: source.revision().get(),
            block_index: u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
            source_start_utf16,
            source_end_utf16,
            info_start_utf16,
            info_end_utf16,
            content_start_utf16,
            content_end_utf16,
            resource_fingerprint: embedded_resource_fingerprint(&source, code.source_range(), kind),
            kind,
            resource_status,
        });
    }
    Ok(encoded)
}

#[cfg(target_os = "macos")]
type MacosVisualRenderPlan = (
    YuStorageVisualRenderPlanSnapshot,
    Vec<YuStorageVisualRenderCommand>,
    Vec<YuStorageVisualRenderPage>,
    Vec<YuStorageVisualRenderDamage>,
);

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
fn macos_render_command_kind_mask(commands: &[RenderCommand]) -> u64 {
    commands.iter().fold(0_u64, |mask, command| {
        let kind = match command {
            RenderCommand::FillRect { .. } => YU_STORAGE_RENDER_COMMAND_FILL_RECT,
            RenderCommand::Glyph { .. } => YU_STORAGE_RENDER_COMMAND_GLYPH,
            RenderCommand::Image { .. } => YU_STORAGE_RENDER_COMMAND_IMAGE,
            RenderCommand::EmbeddedSvg { .. } => YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG,
        };
        mask | (1_u64 << u32::from(kind))
    })
}

#[cfg(target_os = "macos")]
fn macos_render_block_kind_mask(input: &ViewportSceneInput) -> u64 {
    input.blocks().iter().fold(0_u64, |mask, block| {
        let kind = block.kind();
        // Current parser tags are small stable values. Keep one high sentinel
        // for any future tag that cannot be represented by this u64 summary;
        // the native gate will reject it until its renderer is implemented.
        let bit = if kind < 63 {
            1_u64 << u32::from(kind)
        } else {
            1_u64 << 63
        };
        mask | bit
    })
}

#[cfg(target_os = "macos")]
fn macos_render_host_config(
    viewport: ViewportRect,
    size: f32,
    max_width: f32,
    viewport_height: f32,
) -> Result<ViewportRenderConfig, i32> {
    let scene_height = viewport_height.max(1.0);
    let scene_viewport = Rect::new(0.0, viewport.scroll_y(), max_width, scene_height)
        .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    Ok(ViewportRenderConfig::new(
        viewport,
        size,
        scene_viewport,
        Rgba8::black(),
    ))
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
        command_kind_mask: macos_render_command_kind_mask(plan.commands()),
        block_kind_mask: macos_render_block_kind_mask(frame.scene().input()),
    })
}

#[cfg(target_os = "macos")]
fn macos_render_host_frame(
    session: &mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    surface_generation: u64,
) -> Result<YuStorageMacosRenderHostSnapshot, i32> {
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
    let viewport_config = session.session.viewport_config();
    let layout_config = viewport_config.layout();
    if (layout_config.max_width() - max_width).abs() > 0.05
        || (layout_config.line_height() - metrics.line_height()).abs() > 0.05
        || (layout_config.default_advance() - metrics.default_advance()).abs() > 0.05
    {
        return Err(YU_STORAGE_INVALID_VIEWPORT_CONFIG);
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
    let viewport = ViewportRect::new(scroll_y, viewport_height);
    let config = macos_render_host_config(viewport, size, max_width, viewport_height)?;
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
    macos_render_host_snapshot(state, session.session.composition_generation())
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

#[cfg(target_os = "macos")]
fn macos_visual_scene_glyphs(
    session: &mut YuStorageSession,
    host_snapshot: YuStorageMacosRenderHostSnapshot,
) -> Result<
    (
        YuStorageVisualSceneGlyphSnapshot,
        Vec<YuStorageVisualSceneGlyph>,
    ),
    i32,
> {
    let state = session
        .macos_render_host
        .as_ref()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    let publication = state
        .builder
        .last_publication()
        .ok_or(YU_STORAGE_RENDER_HOST_UNAVAILABLE)?;
    if publication.revision().get() != host_snapshot.revision {
        return Err(YU_STORAGE_STALE_REVISION);
    }
    let frame = publication.frame();
    let input = frame.scene().input();
    let scene = frame.scene().scene();
    let config = state.builder.config();
    let source = session.session.snapshot();
    let layout_config = session.session.viewport_config().layout();
    let composition_blocks = session
        .session
        .document()
        .editor()
        .composition_block_range();
    let mut block_metadata = Vec::with_capacity(input.blocks().len());
    {
        let document = session.session.document_mut().editor_mut();
        for block in input.blocks() {
            let glyph_count = if composition_blocks
                .as_ref()
                .is_some_and(|span| span.contains(&block.index()))
            {
                document
                    .block_layout_with_composition_and_shaper(
                        block.index(),
                        layout_config,
                        state.builder.shaper(),
                    )
                    .map_err(status_from_editor_error)?
                    .glyphs()
                    .len()
            } else {
                document
                    .block_layout_with_shaper(block.index(), layout_config, state.builder.shaper())
                    .map_err(status_from_editor_error)?
                    .glyphs()
                    .len()
            };
            let source_start_utf16 = source
                .utf16_offset(block.source().start())
                .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
                .get();
            let source_end_utf16 = source
                .utf16_offset(block.source().end())
                .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
                .get();
            block_metadata.push((
                block.index(),
                source_start_utf16,
                source_end_utf16,
                glyph_count,
            ));
        }
    }

    let mut metadata = block_metadata.into_iter();
    let mut current = metadata.next();
    let mut glyphs = Vec::with_capacity(scene.primitives().len());
    for primitive in scene.primitives().iter().copied() {
        // This ABI is intentionally glyph-only. The retained scene may now
        // also contain solid block fills; skip those primitives without
        // reinterpreting them as glyph metadata. The full render-plan ABI
        // below carries both command kinds.
        let Primitive::Glyph(glyph) = primitive else {
            continue;
        };
        loop {
            match current {
                Some((_, _, _, 0)) => current = metadata.next(),
                Some(_) => break,
                None => return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE),
            }
        }
        let Some((block_index, source_start_utf16, source_end_utf16, glyph_count)) = current else {
            return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE);
        };
        let entry = glyph.atlas();
        let rect = entry.rect();
        let metrics = entry.metrics();
        let bounds = glyph.bounds();
        glyphs.push(YuStorageVisualSceneGlyph {
            revision: host_snapshot.revision,
            block_index: u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
            source_start_utf16,
            source_end_utf16,
            page: entry.page().unwrap_or(YU_STORAGE_RENDER_PAGE_NONE),
            atlas_x: rect.x(),
            atlas_y: rect.y(),
            atlas_width: rect.width(),
            atlas_height: rect.height(),
            origin_x: glyph.origin().x(),
            origin_y: glyph.origin().y(),
            bearing_x: metrics.bearing_x(),
            bearing_y: metrics.bearing_y(),
            advance_x: metrics.advance_x(),
            bounds_x: bounds.x(),
            bounds_y: bounds.y(),
            bounds_width: bounds.width(),
            bounds_height: bounds.height(),
            color_rgba: glyph.color().packed(),
        });
        current = Some((
            block_index,
            source_start_utf16,
            source_end_utf16,
            glyph_count.saturating_sub(1),
        ));
    }
    while let Some((_, _, _, count)) = current {
        if count != 0 {
            return Err(YU_STORAGE_RENDER_HOST_UNAVAILABLE);
        }
        current = metadata.next();
    }

    let block_range = input.block_range();
    let block_start = u64::try_from(block_range.start).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let block_end = u64::try_from(block_range.end).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let glyph_count = u64::try_from(glyphs.len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let snapshot = YuStorageVisualSceneGlyphSnapshot {
        revision: host_snapshot.revision,
        composition_generation: host_snapshot.composition_generation,
        frame_revision: host_snapshot.frame_revision,
        surface_generation: host_snapshot.surface_generation,
        frame_serial: host_snapshot.frame_serial,
        block_start,
        block_end,
        glyph_count,
        content_height: input.content_height(),
        scroll_y: config.viewport().scroll_y(),
        viewport_height: config.viewport().height(),
        max_scroll_y: (input.content_height() - config.viewport().height()).max(0.0),
        viewport_width: config.scene_viewport().width(),
    };
    Ok((snapshot, glyphs))
}

#[cfg(target_os = "macos")]
fn macos_visual_render_plan(
    session: &mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
) -> Result<MacosVisualRenderPlan, i32> {
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
    let (shaper, metrics, _layout_config) = core_text_system_ui_layout(size, max_width)?;
    let viewport_config = session.session.viewport_config();
    let layout_config = viewport_config.layout();
    if (layout_config.max_width() - max_width).abs() > 0.05
        || (layout_config.line_height() - metrics.line_height()).abs() > 0.05
        || (layout_config.default_advance() - metrics.default_advance()).abs() > 0.05
    {
        return Err(YU_STORAGE_INVALID_VIEWPORT_CONFIG);
    }

    let viewport = ViewportRect::new(scroll_y, viewport_height);
    let composition_generation = session.session.composition_generation();
    let document = session.session.document_mut().editor_mut();
    let viewport_snapshot = if document.composition().is_some() {
        document
            .visible_blocks_with_composition_and_shaper(viewport, &shaper)
            .map_err(|error| storage_status(error.into()))?
    } else {
        document
            .visible_blocks_with_shaper(viewport, &shaper)
            .map_err(|error| storage_status(error.into()))?
    };
    let source = document.snapshot();
    let revision = source.revision();
    let config = document.viewport_config().layout();
    let definitions = document.markdown().reference_definitions().clone();
    let rasterizer = shaper.rasterizer();
    let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::default());
    let mut block_glyphs = Vec::with_capacity(viewport_snapshot.blocks().len());
    let mut embedded_requests = Vec::new();
    let composition_blocks = document.composition_block_range();

    for block in viewport_snapshot.blocks() {
        let layout = if composition_blocks
            .as_ref()
            .is_some_and(|span| span.contains(&block.index()))
        {
            document
                .block_layout_with_composition_and_shaper(block.index(), config, &shaper)
                .map_err(status_from_editor_error)?
        } else {
            document
                .block_layout_with_shaper(block.index(), config, &shaper)
                .map_err(status_from_editor_error)?
                .clone()
        };
        for placement in layout.glyphs() {
            let key = GlyphRasterKey::new(placement.face(), placement.glyph(), size)
                .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
            if atlas.entry(key).is_none() {
                let glyph = rasterizer
                    .rasterize(key)
                    .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
                atlas.insert(glyph).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
            }
        }
        block_glyphs.push((
            block.index(),
            block.source(),
            layout.glyphs().len(),
            renderable_image_count(&source, &layout, &definitions),
            viewport_block_background(block.kind()),
        ));
        let projection = document
            .block_projection(block.index())
            .map_err(status_from_editor_error)?
            .clone();
        let BlockProjection::FencedCode(code) = projection else {
            continue;
        };
        let Some(kind) = embedded_resource_kind(&source, code.info_string()) else {
            continue;
        };
        let embedded_kind = embedded_resource_kind_from_ffi(kind).ok_or(YU_STORAGE_EDITOR_ERROR)?;
        let content = embedded_resource_content(&source, code.content())?;
        let content = if content.is_empty() {
            "\n".to_owned()
        } else {
            content
        };
        embedded_requests.push(
            EmbeddedRenderRequest::new(revision, code.source_range(), embedded_kind, content)
                .map_err(|_| YU_STORAGE_EDITOR_ERROR)?,
        );
    }

    let embedded_publications = {
        let embedded_resources = &mut session.macos_embedded_resources;
        let mut publications = Vec::with_capacity(embedded_requests.len());
        for request in embedded_requests {
            if let Some(publication) = embedded_resources.publication_for(request, revision)? {
                publications.push(publication);
            }
        }
        publications
    };

    let scene_height = viewport_snapshot
        .content_height()
        .max(viewport_height)
        .max(1.0);
    // The retained commands keep document-space coordinates.  The native
    // Metal bridge subtracts the render-plan viewport origin when it maps
    // them into the surface, so the plan must carry the current scroll.
    let scene_viewport =
        Rect::new(0.0, scroll_y, max_width, scene_height).map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let mut render_plans = RenderPlanBuilder::new();
    let frame = assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded(
        document,
        viewport,
        ViewportRenderConfig::new(viewport, size, scene_viewport, Rgba8::black()),
        &shaper,
        &atlas,
        &mut render_plans,
        &[],
        &[],
        &embedded_publications,
    )
    .map_err(|_| YU_STORAGE_EDITOR_ERROR)?;
    let plan = frame.plan();
    if plan.revision().get() != expected_revision {
        return Err(YU_STORAGE_STALE_REVISION);
    }

    let mut block_metadata = Vec::new();
    for (block_index, source_range, glyph_count, image_count, background) in &block_glyphs {
        let source_start_utf16 = source
            .utf16_offset(source_range.start())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get();
        let source_end_utf16 = source
            .utf16_offset(source_range.end())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get();
        if background.is_some() {
            block_metadata.push((
                YU_STORAGE_RENDER_COMMAND_FILL_RECT,
                *block_index,
                source_start_utf16,
                source_end_utf16,
            ));
        }
        block_metadata.extend(std::iter::repeat_n(
            (
                YU_STORAGE_RENDER_COMMAND_GLYPH,
                *block_index,
                source_start_utf16,
                source_end_utf16,
            ),
            *glyph_count,
        ));
        block_metadata.extend(std::iter::repeat_n(
            (
                YU_STORAGE_RENDER_COMMAND_IMAGE,
                *block_index,
                source_start_utf16,
                source_end_utf16,
            ),
            *image_count,
        ));
    }
    for (block_index, source_range, _, _, _) in &block_glyphs {
        let embedded_count = embedded_publications
            .iter()
            .filter(|publication| publication.source_range() == *source_range)
            .count();
        let source_start_utf16 = source
            .utf16_offset(source_range.start())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get();
        let source_end_utf16 = source
            .utf16_offset(source_range.end())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?
            .get();
        block_metadata.extend(std::iter::repeat_n(
            (
                YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG,
                *block_index,
                source_start_utf16,
                source_end_utf16,
            ),
            embedded_count,
        ));
    }
    if block_metadata.len() != plan.commands().len() {
        return Err(YU_STORAGE_EDITOR_ERROR);
    }

    let mut commands = Vec::with_capacity(plan.commands().len());
    for (command, metadata) in plan.commands().iter().copied().zip(block_metadata) {
        let (metadata_kind, block_index, source_start_utf16, source_end_utf16) = metadata;
        let block_index = u64::try_from(block_index).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
        let encoded = match command {
            RenderCommand::FillRect { bounds, color } => {
                if metadata_kind != YU_STORAGE_RENDER_COMMAND_FILL_RECT {
                    return Err(YU_STORAGE_EDITOR_ERROR);
                }
                YuStorageVisualRenderCommand {
                    revision: plan.revision().get(),
                    block_index,
                    source_start_utf16,
                    source_end_utf16,
                    kind: YU_STORAGE_RENDER_COMMAND_FILL_RECT,
                    page: YU_STORAGE_RENDER_PAGE_NONE,
                    atlas_x: 0,
                    atlas_y: 0,
                    atlas_width: 0,
                    atlas_height: 0,
                    origin_x: bounds.x(),
                    origin_y: bounds.y(),
                    bearing_x: 0.0,
                    bearing_y: 0.0,
                    advance_x: 0.0,
                    bounds_x: bounds.x(),
                    bounds_y: bounds.y(),
                    bounds_width: bounds.width(),
                    bounds_height: bounds.height(),
                    color_rgba: color.packed(),
                    resource: 0,
                    embedded_generation: 0,
                    embedded_kind: 0,
                    embedded_width: 0,
                    embedded_height: 0,
                }
            }
            RenderCommand::Glyph {
                page,
                rect,
                origin,
                metrics,
                color,
            } => {
                if metadata_kind != YU_STORAGE_RENDER_COMMAND_GLYPH {
                    return Err(YU_STORAGE_EDITOR_ERROR);
                }
                YuStorageVisualRenderCommand {
                    revision: plan.revision().get(),
                    block_index,
                    source_start_utf16,
                    source_end_utf16,
                    kind: YU_STORAGE_RENDER_COMMAND_GLYPH,
                    page: page.unwrap_or(YU_STORAGE_RENDER_PAGE_NONE),
                    atlas_x: rect.x(),
                    atlas_y: rect.y(),
                    atlas_width: rect.width(),
                    atlas_height: rect.height(),
                    origin_x: origin.x(),
                    origin_y: origin.y(),
                    bearing_x: metrics.bearing_x(),
                    bearing_y: metrics.bearing_y(),
                    advance_x: metrics.advance_x(),
                    bounds_x: origin.x() + metrics.bearing_x(),
                    bounds_y: origin.y() - metrics.bearing_y(),
                    bounds_width: rect.width() as f32,
                    bounds_height: rect.height() as f32,
                    color_rgba: color.packed(),
                    resource: 0,
                    embedded_generation: 0,
                    embedded_kind: 0,
                    embedded_width: 0,
                    embedded_height: 0,
                }
            }
            RenderCommand::Image {
                resource,
                bounds,
                fallback,
            } => {
                if metadata_kind != YU_STORAGE_RENDER_COMMAND_IMAGE {
                    return Err(YU_STORAGE_EDITOR_ERROR);
                }
                YuStorageVisualRenderCommand {
                    revision: plan.revision().get(),
                    block_index,
                    source_start_utf16,
                    source_end_utf16,
                    kind: YU_STORAGE_RENDER_COMMAND_IMAGE,
                    page: YU_STORAGE_RENDER_PAGE_NONE,
                    atlas_x: 0,
                    atlas_y: 0,
                    atlas_width: 0,
                    atlas_height: 0,
                    origin_x: 0.0,
                    origin_y: 0.0,
                    bearing_x: 0.0,
                    bearing_y: 0.0,
                    advance_x: 0.0,
                    bounds_x: bounds.x(),
                    bounds_y: bounds.y(),
                    bounds_width: bounds.width(),
                    bounds_height: bounds.height(),
                    color_rgba: fallback.packed(),
                    resource,
                    embedded_generation: 0,
                    embedded_kind: 0,
                    embedded_width: 0,
                    embedded_height: 0,
                }
            }
            RenderCommand::EmbeddedSvg {
                resource,
                generation,
                kind,
                bounds,
                width,
                height,
                fallback,
            } => {
                if metadata_kind != YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG {
                    return Err(YU_STORAGE_EDITOR_ERROR);
                }
                YuStorageVisualRenderCommand {
                    revision: plan.revision().get(),
                    block_index,
                    source_start_utf16,
                    source_end_utf16,
                    kind: YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG,
                    page: YU_STORAGE_RENDER_PAGE_NONE,
                    atlas_x: 0,
                    atlas_y: 0,
                    atlas_width: 0,
                    atlas_height: 0,
                    origin_x: 0.0,
                    origin_y: 0.0,
                    bearing_x: 0.0,
                    bearing_y: 0.0,
                    advance_x: 0.0,
                    bounds_x: bounds.x(),
                    bounds_y: bounds.y(),
                    bounds_width: bounds.width(),
                    bounds_height: bounds.height(),
                    color_rgba: fallback.packed(),
                    resource,
                    embedded_generation: generation,
                    embedded_kind: kind,
                    embedded_width: width,
                    embedded_height: height,
                }
            }
        };
        commands.push(encoded);
    }

    let pages = plan
        .uploads()
        .iter()
        .map(|upload| YuStorageVisualRenderPage {
            revision: plan.revision().get(),
            page: upload.page(),
            width: upload.width(),
            height: upload.height(),
            fingerprint: upload.fingerprint(),
        })
        .collect::<Vec<_>>();
    let damage = plan
        .damage()
        .iter()
        .copied()
        .map(|rect| YuStorageVisualRenderDamage {
            revision: plan.revision().get(),
            x: rect.x(),
            y: rect.y(),
            width: rect.width(),
            height: rect.height(),
        })
        .collect::<Vec<_>>();
    let block_start = u64::try_from(viewport_snapshot.range().start())
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let block_end =
        u64::try_from(viewport_snapshot.range().end()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
    let snapshot = YuStorageVisualRenderPlanSnapshot {
        revision: plan.revision().get(),
        composition_generation,
        block_start,
        block_end,
        command_count: u64::try_from(commands.len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        upload_count: u64::try_from(pages.len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        damage_count: u64::try_from(damage.len()).map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        content_height: viewport_snapshot.content_height(),
        scroll_y,
        viewport_height,
        max_scroll_y: (viewport_snapshot.content_height() - viewport_height).max(0.0),
        viewport_width: max_width,
        embedded_command_count: u64::try_from(
            plan.commands()
                .iter()
                .filter(|command| matches!(command, RenderCommand::EmbeddedSvg { .. }))
                .count(),
        )
        .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        embedded_upload_count: u64::try_from(plan.embedded_uploads().len())
            .map_err(|_| YU_STORAGE_INVALID_SELECTION)?,
        embedded_upload_bytes: {
            let mut total = 0_u64;
            for upload in plan.embedded_uploads() {
                let bytes = u64::try_from(upload.markup().len())
                    .map_err(|_| YU_STORAGE_INVALID_SELECTION)?;
                total = total
                    .checked_add(bytes)
                    .ok_or(YU_STORAGE_INVALID_SELECTION)?;
            }
            total
        },
    };
    Ok((snapshot, commands, pages, damage))
}

/// Returns an owned, count/fill render-plan publication assembled from
/// CoreText-shaped layouts, a CPU glyph atlas and `yu-workspace`'s existing
/// scene/render pipeline. The native side receives command/page/damage scalars
/// only; production TextKit and Metal submission remain separate.
///
/// # Safety
/// All output pointers must be writable when non-null. A non-zero capacity
/// requires the corresponding array pointer to be non-null. On capacity
/// failure no output array is partially written.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_visual_render_plan(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    snapshot: *mut YuStorageVisualRenderPlanSnapshot,
    commands: *mut YuStorageVisualRenderCommand,
    command_capacity: usize,
    pages: *mut YuStorageVisualRenderPage,
    page_capacity: usize,
    damage: *mut YuStorageVisualRenderDamage,
    damage_capacity: usize,
    written_commands: *mut usize,
    written_pages: *mut usize,
    written_damage: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if snapshot.is_null()
        || written_commands.is_null()
        || written_pages.is_null()
        || written_damage.is_null()
    {
        return YU_STORAGE_NULL_POINTER;
    }
    if (command_capacity > 0 && commands.is_null())
        || (page_capacity > 0 && pages.is_null())
        || (damage_capacity > 0 && damage.is_null())
    {
        return YU_STORAGE_NULL_POINTER;
    }
    // Keep the count-query header long enough to validate the matching fill
    // call. Composition updates do not change the canonical Revision, so a
    // Revision-only check would permit a stale capacity/header pair.
    let prior_snapshot = unsafe { *snapshot };
    let is_fill = command_capacity > 0 || page_capacity > 0 || damage_capacity > 0;
    #[cfg(not(target_os = "macos"))]
    let _ = (prior_snapshot, is_fill);
    #[cfg(target_os = "macos")]
    if is_fill
        && let Err(status) = validate_visual_fill_identity(
            &session.session,
            expected_revision,
            prior_snapshot.revision,
            prior_snapshot.composition_generation,
        )
    {
        // SAFETY: output pointers were checked for null and belong to caller.
        unsafe {
            *snapshot = YuStorageVisualRenderPlanSnapshot::default();
            *written_commands = 0;
            *written_pages = 0;
            *written_damage = 0;
        }
        return status;
    }
    // SAFETY: output pointers were checked for null and belong to the caller.
    unsafe {
        *snapshot = YuStorageVisualRenderPlanSnapshot::default();
        *written_commands = 0;
        *written_pages = 0;
        *written_damage = 0;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            commands,
            command_capacity,
            pages,
            page_capacity,
            damage,
            damage_capacity,
        );
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        let (header, encoded_commands, encoded_pages, encoded_damage) =
            match macos_visual_render_plan(
                session,
                expected_revision,
                size,
                max_width,
                scroll_y,
                viewport_height,
            ) {
                Ok(plan) => plan,
                Err(status) => return status,
            };
        // SAFETY: output pointers were checked above and belong to the caller.
        unsafe {
            *snapshot = header;
            *written_commands = encoded_commands.len();
            *written_pages = encoded_pages.len();
            *written_damage = encoded_damage.len();
        }
        if command_capacity == 0
            && commands.is_null()
            && page_capacity == 0
            && pages.is_null()
            && damage_capacity == 0
            && damage.is_null()
        {
            return YU_STORAGE_OK;
        }
        if encoded_commands.len() > command_capacity
            || encoded_pages.len() > page_capacity
            || encoded_damage.len() > damage_capacity
        {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded_commands.is_empty() {
            // SAFETY: capacity was checked against the command count.
            unsafe {
                ptr::copy_nonoverlapping(
                    encoded_commands.as_ptr(),
                    commands,
                    encoded_commands.len(),
                );
            }
        }
        if !encoded_pages.is_empty() {
            // SAFETY: capacity was checked against the page count.
            unsafe {
                ptr::copy_nonoverlapping(encoded_pages.as_ptr(), pages, encoded_pages.len());
            }
        }
        if !encoded_damage.is_empty() {
            // SAFETY: capacity was checked against the damage count.
            unsafe {
                ptr::copy_nonoverlapping(encoded_damage.as_ptr(), damage, encoded_damage.len());
            }
        }
        YU_STORAGE_OK
    }
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
        let value = match macos_render_host_frame(
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_generation,
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
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_generation,
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
                command_kind_mask: host_snapshot.command_kind_mask,
                block_kind_mask: host_snapshot.block_kind_mask,
            };
        }
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
    }
    YU_STORAGE_OK
}

/// Returns the actual glyph primitives from the persistent Rust retained
/// scene. This is an opt-in diagnostic/native-scene bridge: atlas pixels and
/// document/layout objects remain Rust-owned, while the count/fill values are
/// an atomic, Revision-bound snapshot.
///
/// # Safety
/// `session` must be a live handle. `snapshot` and `written` must be writable;
/// `glyphs` must point to `capacity` writable entries when `capacity > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_visual_scene_glyphs(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    surface_generation: u64,
    snapshot: *mut YuStorageVisualSceneGlyphSnapshot,
    glyphs: *mut YuStorageVisualSceneGlyph,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if snapshot.is_null() || written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && glyphs.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    let prior_snapshot = unsafe { *snapshot };
    #[cfg(target_os = "macos")]
    if capacity > 0
        && let Err(status) = validate_visual_fill_identity(
            &session.session,
            expected_revision,
            prior_snapshot.revision,
            prior_snapshot.composition_generation,
        )
    {
        // SAFETY: output pointers were checked for null and belong to caller.
        unsafe {
            *snapshot = YuStorageVisualSceneGlyphSnapshot::default();
            *written = 0;
        }
        return status;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = prior_snapshot;
    // SAFETY: output pointers were checked above and belong to the caller.
    unsafe {
        *snapshot = YuStorageVisualSceneGlyphSnapshot::default();
        *written = 0;
    }

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
            glyphs,
            capacity,
        );
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        let host_snapshot = match macos_render_host_frame(
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            surface_generation,
        ) {
            Ok(snapshot) => snapshot,
            Err(status) => return status,
        };
        let (header, encoded) = match macos_visual_scene_glyphs(session, host_snapshot) {
            Ok(values) => values,
            Err(status) => return status,
        };
        // SAFETY: output pointers were checked above and belong to the caller.
        unsafe {
            *snapshot = header;
            *written = encoded.len();
        }
        if capacity == 0 && glyphs.is_null() {
            return YU_STORAGE_OK;
        }
        if encoded.len() > capacity {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded.is_empty() {
            // SAFETY: capacity was checked against the encoded glyph count.
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), glyphs, encoded.len()) };
        }
        YU_STORAGE_OK
    }
}

/// Returns a count/fill owned scene snapshot assembled by Rust's retained
/// scene boundary. This is intentionally a diagnostic bridge: Swift receives
/// validated rectangle scalars and source ranges, while glyph/image payloads
/// remain in the Rust scene/renderer pipeline until a later phase.
///
/// # Safety
/// `session` must be null or a live handle. `snapshot` and `written` must
/// point to writable values. `primitives` must point to `capacity` writable
/// values when `capacity > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_visual_scene(
    session: *mut YuStorageSession,
    expected_revision: u64,
    size: f32,
    max_width: f32,
    scroll_y: f32,
    viewport_height: f32,
    snapshot: *mut YuStorageVisualSceneSnapshot,
    primitives: *mut YuStorageVisualScenePrimitive,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if snapshot.is_null() || written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && primitives.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: output pointers were checked for null and belong to the caller.
    unsafe {
        *snapshot = YuStorageVisualSceneSnapshot::default();
        *written = 0;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
            primitives,
            capacity,
        );
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        let (header, encoded) = match macos_visual_scene(
            session,
            expected_revision,
            size,
            max_width,
            scroll_y,
            viewport_height,
        ) {
            Ok(scene) => scene,
            Err(status) => return status,
        };
        // SAFETY: output pointers were checked above and belong to the caller.
        unsafe {
            *snapshot = header;
            *written = encoded.len();
        }
        if capacity == 0 && primitives.is_null() {
            return YU_STORAGE_OK;
        }
        if encoded.len() > capacity {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded.is_empty() {
            // SAFETY: capacity was checked against the encoded primitive count.
            unsafe {
                ptr::copy_nonoverlapping(encoded.as_ptr(), primitives, encoded.len());
            }
        }
        YU_STORAGE_OK
    }
}

/// Returns source-backed image metadata for the current Markdown Revision.
/// The count/fill result contains no decoded bytes or native image objects;
/// callers resolve destination ranges through the existing source-range API
/// and schedule decoding on their own worker queue.
///
/// # Safety
/// `session` must be null or a live handle. `snapshot` and `written` must be
/// writable. `images` must point to `capacity` writable entries when
/// `capacity > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_visual_images(
    session: *mut YuStorageSession,
    expected_revision: u64,
    images: *mut YuStorageVisualImage,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && images.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` was checked above and belongs to the caller.
    unsafe { *written = 0 };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session, expected_revision, images, capacity);
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        let encoded = match macos_visual_images(session, expected_revision) {
            Ok(images) => images,
            Err(status) => return status,
        };
        // SAFETY: `written` was checked above and belongs to the caller.
        unsafe { *written = encoded.len() };
        if capacity == 0 && images.is_null() {
            return YU_STORAGE_OK;
        }
        if encoded.len() > capacity {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded.is_empty() {
            // SAFETY: capacity was checked against the encoded image count.
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), images, encoded.len()) };
        }
        YU_STORAGE_OK
    }
}

/// Returns source-backed Math/Mermaid fenced-block metadata for the current
/// Markdown Revision. Requests pass through the session-owned embedded cache;
/// the default host renderer is explicitly unsupported, so native hosts retain
/// their complete source range until a concrete renderer is registered.
///
/// # Safety
/// `session` must be null or a live handle. `embedded` and `written` must be
/// writable. `embedded` must point to `capacity` writable entries when
/// `capacity > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_macos_visual_embedded_resources(
    session: *mut YuStorageSession,
    expected_revision: u64,
    embedded: *mut YuStorageVisualEmbeddedResource,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    if written.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    if capacity > 0 && embedded.is_null() {
        return YU_STORAGE_NULL_POINTER;
    }
    // SAFETY: `written` was checked above and belongs to the caller.
    unsafe { *written = 0 };

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (session, expected_revision, embedded, capacity);
        return YU_STORAGE_CORE_TEXT_UNAVAILABLE;
    }

    #[cfg(target_os = "macos")]
    {
        let encoded = match macos_visual_embedded_resources(session, expected_revision) {
            Ok(resources) => resources,
            Err(status) => return status,
        };
        // SAFETY: `written` was checked above and belongs to the caller.
        unsafe { *written = encoded.len() };
        if capacity == 0 && embedded.is_null() {
            return YU_STORAGE_OK;
        }
        if encoded.len() > capacity {
            return YU_STORAGE_BUFFER_TOO_SMALL;
        }
        if !encoded.is_empty() {
            // SAFETY: capacity was checked against the encoded resource count.
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), embedded, encoded.len()) };
        }
        YU_STORAGE_OK
    }
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
    margin: f32,
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
            margin,
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
        let viewport_config = session.session.viewport_config();
        let layout_config = viewport_config.layout();
        if (layout_config.max_width() - max_width).abs() > 0.05
            || (layout_config.line_height() - metrics.line_height()).abs() > 0.05
            || (layout_config.default_advance() - metrics.default_advance()).abs() > 0.05
        {
            return YU_STORAGE_INVALID_VIEWPORT_CONFIG;
        }
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

/// Returns the number of Revision-bound semantic Accessibility nodes.
///
/// # Safety
/// `session` must be null or a live handle; `output` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_semantic_node_count(
    session: *const YuStorageSession,
    expected_revision: u64,
    output: *mut usize,
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
    let snapshot = match accessibility_semantic_snapshot(&session.session) {
        Ok(snapshot) => snapshot,
        Err(status) => return status,
    };
    // SAFETY: output is checked above and belongs to the caller.
    unsafe { *output = snapshot.nodes().len() };
    YU_STORAGE_OK
}

/// Copies the Revision-bound semantic Accessibility tree in document order.
/// The count/fill convention matches other native owned queries: a null output
/// with zero capacity returns the required node count.
///
/// # Safety
/// `session` must be a live handle. `written` must be writable; `output` must
/// provide `capacity` writable nodes when non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_accessibility_semantic_nodes(
    session: *const YuStorageSession,
    expected_revision: u64,
    output: *mut YuStorageAccessibilityNode,
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
    write_accessibility_nodes(snapshot.nodes(), output, capacity, written)
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

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_cancel_close(session: *mut YuStorageSession) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    session
        .session
        .cancel_close()
        .map(|_| YU_STORAGE_OK)
        .unwrap_or(YU_STORAGE_INVALID_STATE)
}

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_save_close(session: *mut YuStorageSession) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    match session.session.save_close() {
        Ok(_) => YU_STORAGE_OK,
        Err(error) => status_from_error(error),
    }
}

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_discard_close(session: *mut YuStorageSession) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    session
        .session
        .discard_close()
        .map(|_| YU_STORAGE_OK)
        .unwrap_or(YU_STORAGE_INVALID_STATE)
}

/// # Safety
///
/// `session` must be null or a live handle returned by the open function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yu_storage_session_save_failed_external(
    session: *mut YuStorageSession,
    external_state: u8,
) -> i32 {
    let Some(session) = (unsafe { session.as_mut() }) else {
        return YU_STORAGE_NULL_POINTER;
    };
    let state = match external_file_state_from_ffi(external_state) {
        Ok(state) => state,
        Err(status) => return status,
    };
    session
        .session
        .save_failed_external(state)
        .map(|_| YU_STORAGE_OK)
        .unwrap_or(YU_STORAGE_INVALID_STATE)
}

#[cfg(test)]
mod tests {
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
    fn ffi_visual_images_publish_source_ranges_and_reject_stale_revision() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-images-{id}.md"));
        let source = "![logo](assets/yu.png)\n\n![mark][asset]\n\n[asset]: icons/yu.png\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_images(raw, 0, ptr::null_mut(), 0, &mut required)
            },
            YU_STORAGE_OK
        );
        assert_eq!(required, 2);
        let mut images = vec![YuStorageVisualImage::default(); required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_images(
                    raw,
                    0,
                    images.as_mut_ptr(),
                    images.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, 2);
        assert_eq!(images[0].kind, YU_STORAGE_IMAGE_INLINE);
        assert_eq!(images[1].kind, YU_STORAGE_IMAGE_REFERENCE);
        assert!(images.iter().all(|image| image.revision == 0));
        assert!(images.iter().all(|image| image.resource_fingerprint != 0));
        assert!(
            images
                .iter()
                .all(|image| image.resource_status == YU_STORAGE_IMAGE_RESOURCE_UNKNOWN)
        );
        assert_eq!(
            images[1].destination_end_utf16 - images[1].destination_start_utf16,
            "icons/yu.png".encode_utf16().count() as u64
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"x".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        let mut stale_written = 99;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_images(
                    raw,
                    0,
                    images.as_mut_ptr(),
                    images.len(),
                    &mut stale_written,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(stale_written, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_visual_embedded_resources_publish_math_and_mermaid_ranges() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-embedded-{id}.md"));
        let source = "```mermaid\nflowchart TD\n    A --> B\n```\n\n```math\nx^2 + y^2\n```\n\n```rust\nfn main() {}\n```\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_embedded_resources(
                    raw,
                    0,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(required, 2);
        assert_eq!(
            unsafe { (*raw).macos_embedded_resources.cache.failure_count() },
            1
        );
        let mut resources = vec![YuStorageVisualEmbeddedResource::default(); required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_embedded_resources(
                    raw,
                    0,
                    resources.as_mut_ptr(),
                    resources.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, 2);
        assert_eq!(resources[0].kind, YU_STORAGE_EMBEDDED_MERMAID);
        assert_eq!(resources[1].kind, YU_STORAGE_EMBEDDED_MATH);
        assert!(resources.iter().all(|resource| resource.revision == 0));
        assert!(
            resources
                .iter()
                .all(|resource| resource.resource_fingerprint != 0)
        );
        assert_eq!(
            resources[0].resource_status,
            YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED
        );
        assert_eq!(
            resources[1].resource_status,
            YU_STORAGE_EMBEDDED_RESOURCE_READY
        );
        let mermaid_source = source.find("```mermaid").expect("mermaid source");
        let mermaid_end = source[mermaid_source..]
            .find("```\n\n")
            .map(|end| mermaid_source + end + 4)
            .expect("mermaid fence");
        assert_eq!(
            resources[0].source_end_utf16 - resources[0].source_start_utf16,
            source[mermaid_source..mermaid_end].encode_utf16().count() as u64
        );
        assert!(resources[0].content_end_utf16 > resources[0].content_start_utf16);
        assert!(resources[1].info_end_utf16 > resources[1].info_start_utf16);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"x".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        let mut stale_written = 99;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_embedded_resources(
                    raw,
                    0,
                    resources.as_mut_ptr(),
                    resources.len(),
                    &mut stale_written,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(stale_written, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_visual_embedded_resources_keep_empty_body_on_failed_path() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-empty-embedded-{id}.md"));
        let source = "```math\n```\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_embedded_resources(
                    raw,
                    0,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(required, 1);
        let mut resource = YuStorageVisualEmbeddedResource::default();
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_embedded_resources(
                    raw,
                    0,
                    &mut resource,
                    1,
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, 1);
        assert_eq!(
            resource.resource_status,
            YU_STORAGE_EMBEDDED_RESOURCE_FAILED
        );
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_visual_render_plan_consumes_published_math_svg() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-embedded-plan-{id}.md"));
        fs::write(&path, "```math\nx^2 + y^2\n```\n").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );
        let (snapshot, commands, _, _) = macos_visual_render_plan(
            unsafe { raw.as_mut() }.expect("session"),
            0,
            14.0,
            500.0,
            0.0,
            240.0,
        )
        .expect("embedded render plan");
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.embedded_command_count, 1);
        assert_eq!(snapshot.embedded_upload_count, 1);
        assert!(snapshot.embedded_upload_bytes > 0);
        let embedded = commands
            .iter()
            .find(|command| command.kind == YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG)
            .expect("embedded command");
        assert_ne!(embedded.resource, 0);
        assert_eq!(embedded.embedded_kind, YU_STORAGE_EMBEDDED_MATH);
        assert_eq!(embedded.embedded_generation, 1);
        assert!(embedded.embedded_width > 0);
        assert!(embedded.embedded_height > 0);
        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
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

        let mut ordered = YuStorageSelection::default();
        assert_eq!(
            unsafe { yu_storage_session_selection(raw, &mut ordered) },
            YU_STORAGE_OK
        );
        assert_eq!(ordered.start_utf16, 0);
        assert_eq!(ordered.end_utf16, end);

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

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_projected_source(raw, 0, ptr::null_mut(), 0, &mut required)
            },
            YU_STORAGE_OK
        );
        let mut projected = vec![0_u8; required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_projected_source(
                    raw,
                    0,
                    projected.as_mut_ptr(),
                    projected.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, required);
        let projected = String::from_utf8(projected).expect("projected UTF-8");
        assert_eq!(projected, "羽 链接 🙂\n");

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
                yu_storage_session_projected_source(raw, 0, ptr::null_mut(), 0, &mut required)
            },
            YU_STORAGE_STALE_REVISION
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
    fn ffi_block_projection_is_revision_bound_and_parser_owned() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-block-projection-{id}.md"));
        let source = "# 标题\n\n段落 **粗体** 和 [链接](https://example.com)。\n\n- [ ] 任务\n\n> 引用\n\n1. 有序\n\n| A | B |\n| --- | :---: |\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut count = 0;
        assert_eq!(
            unsafe { yu_storage_session_projection_block_count(raw, 0, &mut count) },
            YU_STORAGE_OK
        );
        assert!(count >= 7);
        let mut previous_end = 0_u64;
        let mut seen_kinds = Vec::new();
        let mut seen_projection_kinds = Vec::new();
        let mut projected_blocks = Vec::new();
        for index in 0..count {
            let mut metadata = YuStorageProjectionBlock::default();
            let mut required = 0;
            assert_eq!(
                unsafe {
                    yu_storage_session_projected_block(
                        raw,
                        0,
                        index as u64,
                        &mut metadata,
                        ptr::null_mut(),
                        0,
                        &mut required,
                    )
                },
                YU_STORAGE_OK
            );
            assert_eq!(metadata.revision, 0);
            assert_eq!(metadata.block_index, index as u64);
            assert!(metadata.source_start_utf16 <= metadata.source_end_utf16);
            assert!(metadata.source_start_utf16 >= previous_end);
            previous_end = metadata.source_end_utf16;
            assert_eq!(required, metadata.visual_utf8_length as usize);
            let mut projected = vec![0_u8; required];
            let mut written = 0;
            assert_eq!(
                unsafe {
                    yu_storage_session_projected_block(
                        raw,
                        0,
                        index as u64,
                        &mut metadata,
                        projected.as_mut_ptr(),
                        projected.len(),
                        &mut written,
                    )
                },
                YU_STORAGE_OK
            );
            assert_eq!(written, required);
            assert_eq!(metadata.visual_utf8_length as usize, projected.len());
            assert_eq!(
                metadata.visual_utf16_length as usize,
                String::from_utf8_lossy(&projected).encode_utf16().count()
            );
            seen_kinds.push(metadata.kind);
            seen_projection_kinds.push(metadata.projection_kind);
            projected_blocks.push(String::from_utf8(projected).expect("projected UTF-8"));
        }
        assert!(seen_kinds.contains(&YU_STORAGE_PROJECTION_BLOCK_HEADING));
        assert!(seen_kinds.contains(&YU_STORAGE_PROJECTION_BLOCK_BLOCK_QUOTE));
        assert!(seen_kinds.contains(&YU_STORAGE_PROJECTION_BLOCK_LIST_ITEM));
        assert!(seen_kinds.contains(&YU_STORAGE_PROJECTION_BLOCK_TASK_LIST_ITEM));
        assert!(seen_kinds.contains(&YU_STORAGE_PROJECTION_BLOCK_FENCED_CODE));
        assert!(seen_projection_kinds.contains(&YU_STORAGE_PROJECTION_HEADING));
        assert!(seen_projection_kinds.contains(&YU_STORAGE_PROJECTION_BLOCK_QUOTE));
        assert!(seen_projection_kinds.contains(&YU_STORAGE_PROJECTION_LIST));
        assert!(seen_projection_kinds.contains(&YU_STORAGE_PROJECTION_TASK_LIST));
        assert!(seen_projection_kinds.contains(&YU_STORAGE_PROJECTION_FENCED_CODE));
        assert!(seen_projection_kinds.contains(&YU_STORAGE_PROJECTION_TABLE));
        assert!(projected_blocks.iter().any(|text| text.contains("粗体")));
        assert!(projected_blocks.iter().any(|text| text.contains("链接")));
        assert!(projected_blocks.iter().any(|text| text.contains("任务")));
        assert!(projected_blocks.iter().any(|text| text.contains("fn main")));
        assert!(
            projected_blocks
                .iter()
                .all(|text| !text.contains("**粗体**"))
        );
        assert!(
            projected_blocks
                .iter()
                .all(|text| !text.contains("[链接](https://example.com)"))
        );

        let table_index = seen_projection_kinds
            .iter()
            .position(|kind| *kind == YU_STORAGE_PROJECTION_TABLE)
            .expect("table projection should be present");
        assert_eq!(projected_blocks[table_index], "AB12");
        let mut table_count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_projected_table_cells(
                    raw,
                    0,
                    table_index as u64,
                    ptr::null_mut(),
                    0,
                    &mut table_count,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(table_count, 6);
        let mut cells = vec![YuStorageTableCellRange::default(); table_count];
        let mut written_cells = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_projected_table_cells(
                    raw,
                    0,
                    table_index as u64,
                    cells.as_mut_ptr(),
                    cells.len(),
                    &mut written_cells,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written_cells, 6);
        assert_eq!(
            cells.iter().map(|cell| cell.row).collect::<Vec<_>>(),
            [0, 0, 1, 1, 2, 2]
        );
        assert_eq!(
            cells.iter().map(|cell| cell.column).collect::<Vec<_>>(),
            [0, 1, 0, 1, 0, 1]
        );

        let mut layout_count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    ptr::null_mut(),
                    0,
                    &mut layout_count,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(layout_count, 4);
        let mut layout_cells = vec![YuStorageTableLayoutCell::default(); layout_count];
        let mut written_layout = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    layout_cells.as_mut_ptr(),
                    layout_cells.len(),
                    &mut written_layout,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written_layout, 4);
        assert_eq!(
            layout_cells.iter().map(|cell| cell.row).collect::<Vec<_>>(),
            [0, 0, 1, 1]
        );
        assert_eq!(
            layout_cells
                .iter()
                .map(|cell| cell.column)
                .collect::<Vec<_>>(),
            [0, 1, 0, 1]
        );
        assert_eq!(layout_cells[0].x, 0.0);
        assert_eq!(layout_cells[0].y, 0.0);
        assert_eq!(layout_cells[2].y, 2.0);
        assert_eq!(layout_cells[1].alignment, YU_STORAGE_TABLE_ALIGNMENT_CENTER);
        assert!(
            layout_cells
                .iter()
                .all(|cell| cell.source_start_utf16 <= cell.source_end_utf16)
        );

        let mut resized_layout_count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells_with_resize(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    YU_STORAGE_TABLE_RESIZE_COLUMN,
                    0,
                    1.0,
                    ptr::null_mut(),
                    0,
                    &mut resized_layout_count,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(resized_layout_count, 4);
        let mut resized_layout_cells =
            vec![YuStorageTableLayoutCell::default(); resized_layout_count];
        let mut written_resized_layout = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells_with_resize(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    YU_STORAGE_TABLE_RESIZE_COLUMN,
                    0,
                    1.0,
                    resized_layout_cells.as_mut_ptr(),
                    resized_layout_cells.len(),
                    &mut written_resized_layout,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written_resized_layout, 4);
        assert_eq!(resized_layout_cells[0].width, 4.0);
        assert_eq!(resized_layout_cells[1].x, 4.0);
        assert_eq!(resized_layout_cells[1].width, 2.0);
        assert_eq!(resized_layout_cells[2].x, 0.0);
        assert_eq!(resized_layout_cells[3].x, 4.0);
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells_with_resize(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    YU_STORAGE_TABLE_RESIZE_ROW,
                    0,
                    1.0,
                    ptr::null_mut(),
                    0,
                    &mut resized_layout_count,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells_with_resize(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    YU_STORAGE_TABLE_RESIZE_COLUMN,
                    0,
                    f32::NAN,
                    ptr::null_mut(),
                    0,
                    &mut resized_layout_count,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );

        let mut hit = YuStorageTableCellHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_cell_hit_test(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    3.5,
                    2.5,
                    &mut hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(hit.row, 1);
        assert_eq!(hit.column, 1);
        assert_eq!(hit.x, 3.0);
        assert_eq!(hit.y, 2.0);
        assert!(hit.source_start_utf16 < hit.source_end_utf16);
        assert_eq!(
            unsafe {
                yu_storage_session_table_cell_hit_test(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    99.0,
                    99.0,
                    &mut hit,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );

        let mut resize_hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_hit_test(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    3.1,
                    0.5,
                    0.2,
                    &mut resize_hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(resize_hit.revision, 0);
        assert_eq!(resize_hit.block_index, table_index as u64);
        assert_eq!(resize_hit.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert_eq!(resize_hit.index, 0);
        assert_eq!(resize_hit.position, 3.0);
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_hit_test(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    1.0,
                    2.1,
                    0.2,
                    &mut resize_hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(resize_hit.kind, YU_STORAGE_TABLE_RESIZE_ROW);
        assert_eq!(resize_hit.index, 0);
        assert_eq!(resize_hit.position, 2.0);
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_hit_test(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    3.1,
                    0.0,
                    0.0,
                    &mut resize_hit,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(resize_hit, YuStorageTableResizeHit::default());

        let mut metadata = YuStorageProjectionBlock::default();
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_projected_block(
                    raw,
                    0,
                    count as u64,
                    &mut metadata,
                    ptr::null_mut(),
                    0,
                    &mut written,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"x".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_projection_block_count(raw, 0, &mut count) },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_projected_block(
                    raw,
                    0,
                    0,
                    &mut metadata,
                    ptr::null_mut(),
                    0,
                    &mut written,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_projected_table_cells(
                    raw,
                    0,
                    table_index as u64,
                    ptr::null_mut(),
                    0,
                    &mut table_count,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    ptr::null_mut(),
                    0,
                    &mut layout_count,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_layout_cells_with_resize(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    YU_STORAGE_TABLE_RESIZE_COLUMN,
                    0,
                    1.0,
                    ptr::null_mut(),
                    0,
                    &mut resized_layout_count,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_cell_hit_test(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    3.5,
                    2.5,
                    &mut hit,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_hit_test(
                    raw,
                    0,
                    table_index as u64,
                    20.0,
                    2.0,
                    1.0,
                    3.1,
                    0.5,
                    0.2,
                    &mut resize_hit,
                )
            },
            YU_STORAGE_STALE_REVISION
        );

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

        let mut hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_begin(
                    raw, 0, 0, 20.0, 2.0, 1.0, 3.1, 0.5, 0.2, 3.1, &mut hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(hit.kind, YU_STORAGE_TABLE_RESIZE_COLUMN);
        assert_eq!(hit.index, 0);
        assert_eq!(hit.position, 3.0);

        let mut second_hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_table_resize_begin(
                    raw,
                    0,
                    0,
                    20.0,
                    2.0,
                    1.0,
                    3.1,
                    0.5,
                    0.2,
                    3.1,
                    &mut second_hit,
                )
            },
            YU_STORAGE_INVALID_STATE
        );
        assert_eq!(second_hit, YuStorageTableResizeHit::default());

        let mut preview = YuStorageTableResizeCommit::default();
        assert_eq!(
            unsafe { yu_storage_session_table_resize_update(raw, 0, 4.1, &mut preview) },
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
            unsafe { yu_storage_session_table_resize_finish(raw, 0, &mut committed) },
            YU_STORAGE_OK
        );
        assert_eq!(committed, preview);
        assert_eq!(
            unsafe { yu_storage_session_table_resize_update(raw, 0, 4.1, &mut preview) },
            YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE
        );
        assert_eq!(preview, YuStorageTableResizeCommit::default());
        assert_eq!(
            unsafe { yu_storage_session_table_resize_cancel(raw, 0) },
            YU_STORAGE_OK
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"x".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_table_resize_finish(raw, 0, &mut committed) },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(committed, YuStorageTableResizeCommit::default());
        assert_eq!(
            unsafe { yu_storage_session_table_resize_cancel(raw, 1) },
            YU_STORAGE_OK
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_projection_selection_and_hit_test_round_trip_visual_coordinates() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-projection-hit-test-{id}.md"));
        let source = "before **粗体** after\nnext";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let marker_start = source.find("**粗体**").expect("strong range");
        let marker_end = marker_start + "**粗体**".len();
        let source_start_utf16 = source[..marker_start].encode_utf16().count() as u64;
        let source_end_utf16 = source[..marker_end].encode_utf16().count() as u64;
        let mut selection = YuStorageProjectionSelection::default();
        assert_eq!(
            unsafe {
                yu_storage_session_projection_selection(
                    raw,
                    0,
                    source_start_utf16,
                    source_end_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut selection,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(selection.revision, 0);
        assert_eq!(selection.source_start_utf16, source_start_utf16);
        assert_eq!(selection.source_end_utf16, source_end_utf16);
        assert_eq!(selection.visual_start_utf16, 7);
        assert_eq!(selection.visual_end_utf16, 9);
        assert_eq!(selection.round_trip_source_start_utf16, source_start_utf16);
        assert_eq!(selection.round_trip_source_end_utf16, source_end_utf16);

        assert_eq!(
            unsafe {
                yu_storage_session_projection_selection(
                    raw,
                    0,
                    source_end_utf16,
                    source_start_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut selection,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(selection.visual_end_utf16, 0);

        let mut hit = YuStorageProjectionHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_projection_hit_test(raw, 0, 7.1, 0.0, 80.0, 1.0, 1.0, &mut hit)
            },
            YU_STORAGE_OK
        );
        assert_eq!(hit.revision, 0);
        assert_eq!(hit.source_utf16, source_start_utf16);
        assert_eq!(hit.visual_utf16, 7);
        assert_eq!(hit.round_trip_source_utf16, source_start_utf16);
        assert_eq!(
            hit.image_source_start_utf16,
            YU_STORAGE_IMAGE_DESTINATION_NONE
        );
        assert_eq!(
            hit.image_source_end_utf16,
            YU_STORAGE_IMAGE_DESTINATION_NONE
        );
        assert_eq!(hit.line, 0);
        assert_eq!(hit.x, 7.0);
        assert_eq!(hit.y, 0.0);
        assert_eq!(hit.affinity, YU_STORAGE_CARET_AFFINITY_UPSTREAM);

        assert_eq!(
            unsafe {
                yu_storage_session_projection_hit_test(
                    raw,
                    0,
                    f32::NAN,
                    0.0,
                    80.0,
                    1.0,
                    1.0,
                    &mut hit,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(hit.visual_utf16, 0);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_projection_selection(
                    raw,
                    0,
                    source_start_utf16,
                    source_end_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut selection,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_projection_hit_test(raw, 0, 7.1, 0.0, 80.0, 1.0, 1.0, &mut hit)
            },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn ffi_projection_hit_test_exposes_image_source_range() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-image-hit-test-{id}.md"));
        let source = "![alt](image.png)";
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
                yu_storage_session_projection_hit_test(raw, 0, 1.0, 0.0, 80.0, 10.0, 1.0, &mut hit)
            },
            YU_STORAGE_OK
        );
        assert_eq!(hit.revision, 0);
        assert_eq!(hit.image_source_start_utf16, 0);
        assert_eq!(
            hit.image_source_end_utf16,
            source.encode_utf16().count() as u64
        );
        assert_eq!(hit.source_utf16, 0);
        assert_eq!(hit.visual_utf16, 0);

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

        let mut metrics = YuStorageMacosFontMetrics::default();
        assert_eq!(
            unsafe { yu_storage_session_macos_font_metrics(raw, 0, 14.0, 500.0, &mut metrics) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height,
                    metrics.default_advance,
                    metrics.line_height,
                    0.0,
                )
            },
            YU_STORAGE_OK
        );
        let first_line_end = source.find('\n').expect("line ending") as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection(
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

        let mut metrics = YuStorageMacosFontMetrics::default();
        assert_eq!(
            unsafe { yu_storage_session_macos_font_metrics(raw, 0, 14.0, 500.0, &mut metrics) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height,
                    metrics.default_advance,
                    metrics.line_height,
                    0.0,
                )
            },
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

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_projected_source(raw, 0, ptr::null_mut(), 0, &mut required)
            },
            YU_STORAGE_OK
        );
        let mut projected = vec![0_u8; required];
        assert_eq!(
            unsafe {
                yu_storage_session_projected_source(
                    raw,
                    0,
                    projected.as_mut_ptr(),
                    projected.len(),
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        let projected = String::from_utf8(projected).expect("projected UTF-8");
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

        let mut caret = YuStorageProjectionSourceCaret {
            revision: 99,
            ..YuStorageProjectionSourceCaret::default()
        };
        assert_eq!(
            unsafe {
                yu_storage_session_projection_source_caret(
                    raw,
                    0,
                    visual_start_utf16,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                    &mut caret,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(caret.revision, 0);
        assert_eq!(caret.visual_utf16, visual_start_utf16);
        assert_eq!(caret.source_utf16, source_start_utf16);
        assert_eq!(caret.round_trip_visual_utf16, visual_start_utf16);

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

        let emoji_visual_start = projected.find("🙂").expect("emoji") as u64;
        let emoji_utf16 = projected[..emoji_visual_start as usize]
            .encode_utf16()
            .count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_projection_source_caret(
                    raw,
                    0,
                    emoji_utf16 + 1,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut caret,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(caret, YuStorageProjectionSourceCaret::default());

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

    #[test]
    fn ffi_block_layout_is_revision_bound_and_reports_metrics_geometry() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-block-layout-{id}.md"));
        let source = "# 标题\n\n段落 **粗体**";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut layout = YuStorageBlockLayout::default();
        assert_eq!(
            unsafe { yu_storage_session_block_layout(raw, 0, 2, 80.0, 2.0, 1.5, &mut layout) },
            YU_STORAGE_OK
        );
        assert_eq!(layout.revision, 0);
        assert_eq!(layout.block_index, 2);
        assert_eq!(layout.kind, YU_STORAGE_PROJECTION_BLOCK_PARAGRAPH);
        assert_eq!(layout.shaped, 0);
        assert_eq!(layout.line_count, 1);
        assert_eq!(layout.height, 2.0);
        assert_eq!(layout.line_height, 2.0);
        assert_eq!(layout.default_advance, 1.5);
        assert!(layout.visual_utf16_length > 0);
        assert!(layout.width > 0.0);

        assert_eq!(
            unsafe { yu_storage_session_block_layout(raw, 0, 2, 80.0, 2.0, 0.0, &mut layout) },
            YU_STORAGE_EDITOR_ERROR
        );
        assert_eq!(layout.visual_utf16_length, 0);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_block_layout(raw, 0, 2, 80.0, 2.0, 1.5, &mut layout) },
            YU_STORAGE_STALE_REVISION
        );

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_block_layout_and_caret_are_revision_bound() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-block-layout-{id}.md"));
        let source = "# 标题\n\nParagraph **粗体** and 日本語🙂\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut layout = YuStorageBlockLayout::default();
        assert_eq!(
            unsafe { yu_storage_session_macos_block_layout(raw, 0, 2, 14.0, 500.0, &mut layout) },
            YU_STORAGE_OK
        );
        assert_eq!(layout.revision, 0);
        assert_eq!(layout.block_index, 2);
        assert_eq!(layout.shaped, 1);
        assert!(layout.line_count >= 1);
        assert!(layout.height > 0.0);
        assert!(layout.line_height > 0.0);
        assert!(layout.default_advance > 0.0);

        let source_start = source.find("**粗体**").expect("strong marker");
        let source_utf16 = source[..source_start].encode_utf16().count() as u64;
        let mut caret = YuStorageBlockCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_block_caret(
                    raw,
                    0,
                    2,
                    source_utf16,
                    YU_STORAGE_CARET_AFFINITY_UPSTREAM,
                    14.0,
                    500.0,
                    &mut caret,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(caret.revision, 0);
        assert_eq!(caret.block_index, 2);
        assert_eq!(caret.source_utf16, source_utf16);
        assert_eq!(caret.shaped, 1);
        assert!(caret.caret_x.is_finite());
        assert!(caret.caret_y.is_finite());
        assert_eq!(caret.caret_height, layout.line_height);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_macos_block_layout(raw, 0, 2, 14.0, 500.0, &mut layout) },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(
            unsafe {
                yu_storage_session_macos_block_caret(
                    raw,
                    0,
                    2,
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

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_font_metrics_support_empty_documents() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-empty-metrics-{id}.md"));
        fs::write(&path, "").expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let mut metrics = YuStorageMacosFontMetrics::default();
        assert_eq!(
            unsafe { yu_storage_session_macos_font_metrics(raw, 0, 14.0, 500.0, &mut metrics) },
            YU_STORAGE_OK
        );
        assert_eq!(metrics.revision, 0);
        assert_eq!(metrics.size, 14.0);
        assert!(metrics.line_height > 0.0);
        assert!(metrics.default_advance > 0.0);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe { yu_storage_session_macos_font_metrics(raw, 0, 14.0, 500.0, &mut metrics) },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(metrics, YuStorageMacosFontMetrics::default());

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_shaped_viewport_is_count_fill_and_revision_bound() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-viewport-{id}.md"));
        let source = "# 标题\n\nParagraph **粗体** and 日本語🙂\n\n- [ ] 任务\n\n```rust\nfn main() {}\n```\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );

        let mut snapshot = YuStorageShapedViewportSnapshot::default();
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_shaped_viewport_blocks(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert!(required >= 5);
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.block_start, 0);
        assert_eq!(snapshot.block_end, required as u64);
        assert!(snapshot.content_height >= metrics.line_height());
        assert_eq!(snapshot.scroll_y, 0.0);
        assert_eq!(snapshot.viewport_height, 1_000.0);
        assert_eq!(snapshot.max_scroll_y, 0.0);

        let mut too_small = vec![YuStorageShapedViewportBlock::default(); required - 1];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_shaped_viewport_blocks(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    too_small.as_mut_ptr(),
                    too_small.len(),
                    &mut written,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );
        assert_eq!(written, required);
        assert!(
            too_small
                .iter()
                .all(|block| *block == YuStorageShapedViewportBlock::default())
        );

        let mut blocks = vec![YuStorageShapedViewportBlock::default(); required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_shaped_viewport_blocks(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    blocks.as_mut_ptr(),
                    blocks.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, required);
        assert!(blocks.windows(2).all(|pair| {
            pair[0].block_index < pair[1].block_index
                && pair[0].source_end_utf16 <= pair[1].source_start_utf16
                && pair[0].y < pair[1].y
        }));
        assert!(blocks.iter().all(|block| {
            block.revision == 0
                && block.height > 0.0
                && block.y.is_finite()
                && block.height.is_finite()
        }));
        assert!(
            blocks
                .iter()
                .any(|block| block.kind == YU_STORAGE_PROJECTION_BLOCK_HEADING)
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.kind == YU_STORAGE_PROJECTION_BLOCK_TASK_LIST_ITEM)
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.kind == YU_STORAGE_PROJECTION_BLOCK_FENCED_CODE)
        );

        let scrolled_height = metrics.line_height() * 2.0;
        let scrolled_y = metrics.line_height();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_shaped_viewport_blocks(
                    raw,
                    0,
                    14.0,
                    500.0,
                    scrolled_y,
                    scrolled_height,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(snapshot.scroll_y, scrolled_y);
        assert_eq!(snapshot.viewport_height, scrolled_height);
        assert!(snapshot.max_scroll_y > 0.0);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_macos_shaped_viewport_blocks(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(snapshot, YuStorageShapedViewportSnapshot::default());

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_visual_decorations_are_shaped_count_fill_and_generation_bound() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("yu-storage-ffi-macos-visual-decorations-{id}.md"));
        let source = "# 标题\n\nParagraph **粗体** and 日本語🙂\n\nsecond line\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );
        let selection_start = source.find("Paragraph").expect("selection start");
        let selection_end = source.find("日本語").expect("selection end") + "日本語".len();
        let start_utf16 = source[..selection_start].encode_utf16().count() as u64;
        let end_utf16 = source[..selection_end].encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection(
                    raw,
                    0,
                    start_utf16,
                    end_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        let mut snapshot = YuStorageMacosVisualDecorationSnapshot::default();
        let mut caret = YuStorageMacosVisualDecorationCaret::default();
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_decorations(
                    raw,
                    0,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    &mut caret,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert!(required > 0);
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.composition_generation, 0);
        assert_eq!(snapshot.selection_count, required as u64);
        assert_eq!(snapshot.caret_present, 1);
        assert_eq!(caret.present, 1);
        assert!(caret.x.is_finite() && caret.y.is_finite());

        let mut too_small = vec![YuStorageMacosVisualDecorationRect::default(); required - 1];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_decorations(
                    raw,
                    0,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    &mut caret,
                    too_small.as_mut_ptr(),
                    too_small.len(),
                    &mut written,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );
        assert_eq!(written, required);
        assert!(
            too_small
                .iter()
                .all(|rect| *rect == YuStorageMacosVisualDecorationRect::default())
        );

        let mut rects = vec![YuStorageMacosVisualDecorationRect::default(); required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_decorations(
                    raw,
                    0,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    &mut caret,
                    rects.as_mut_ptr(),
                    rects.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, required);
        assert!(rects.iter().all(|rect| {
            rect.revision == 0
                && rect.width > 0.0
                && rect.height > 0.0
                && rect.x.is_finite()
                && rect.y.is_finite()
        }));

        assert_eq!(
            unsafe {
                yu_storage_session_begin_composition(
                    raw,
                    0,
                    start_utf16,
                    end_utf16,
                    "日".as_ptr(),
                    "日".len(),
                    0,
                    1,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_decorations(
                    raw,
                    0,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    &mut caret,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        assert_eq!(snapshot, YuStorageMacosVisualDecorationSnapshot::default());
        assert_eq!(caret, YuStorageMacosVisualDecorationCaret::default());
        assert_eq!(required, 0);
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_decorations(
                    raw,
                    0,
                    1,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    &mut caret,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.composition_generation, 1);
        assert_eq!(snapshot.selection_count, required as u64);
        assert!(required > 0);
        assert_eq!(snapshot.caret_present, 1);
        assert_eq!(caret.present, 1);
        assert!(caret.x.is_finite() && caret.y.is_finite());
        let mut composition_rects = vec![YuStorageMacosVisualDecorationRect::default(); required];
        let mut composition_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_decorations(
                    raw,
                    0,
                    1,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    &mut caret,
                    composition_rects.as_mut_ptr(),
                    composition_rects.len(),
                    &mut composition_written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(composition_written, required);
        assert!(composition_rects.iter().all(|rect| {
            rect.revision == 0
                && rect.width > 0.0
                && rect.height > 0.0
                && rect.x.is_finite()
                && rect.y.is_finite()
        }));
        assert_eq!(
            unsafe { yu_storage_session_cancel_composition(raw, 0, 1) },
            YU_STORAGE_OK
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_decorations(
                    raw,
                    0,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    &mut caret,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(snapshot, YuStorageMacosVisualDecorationSnapshot::default());
        assert_eq!(caret, YuStorageMacosVisualDecorationCaret::default());
        assert_eq!(required, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_visual_scene_is_owned_count_fill_and_revision_bound() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-scene-{id}.md"));
        let source = "# 标题\n\nParagraph **粗体** and 日本語🙂\n\n- [ ] 任务\n\n```rust\nfn main() {}\n```\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );

        let mut snapshot = YuStorageVisualSceneSnapshot::default();
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert!(required >= 2);
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.primitive_count, required as u64);
        assert_eq!(snapshot.viewport_width, 500.0);
        assert_eq!(
            snapshot.block_end - snapshot.block_start,
            (required / 2) as u64
        );

        let mut too_small = vec![YuStorageVisualScenePrimitive::default(); required - 1];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    too_small.as_mut_ptr(),
                    too_small.len(),
                    &mut written,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );
        assert_eq!(written, required);
        assert!(
            too_small
                .iter()
                .all(|primitive| *primitive == YuStorageVisualScenePrimitive::default())
        );

        let mut primitives = vec![YuStorageVisualScenePrimitive::default(); required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    primitives.as_mut_ptr(),
                    primitives.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, required);
        assert!(primitives.windows(2).step_by(2).all(|pair| {
            pair[0].kind == YU_STORAGE_SCENE_PRIMITIVE_BACKGROUND
                && pair[1].kind == YU_STORAGE_SCENE_PRIMITIVE_TEXT_BOUNDS
                && pair[0].revision == 0
                && pair[1].revision == 0
                && pair[0].block_index == pair[1].block_index
                && pair[0].source_start_utf16 == pair[1].source_start_utf16
                && pair[0].source_end_utf16 == pair[1].source_end_utf16
                && pair[0].width == 500.0
                && pair[0].height > 0.0
                && pair[1].y >= pair[0].y
                && pair[1].width <= pair[0].width
        }));
        assert!(
            primitives.windows(2).all(|pair| {
                pair[0].y <= pair[1].y || pair[0].block_index == pair[1].block_index
            })
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(snapshot, YuStorageVisualSceneSnapshot::default());
        assert_eq!(required, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_visual_render_plan_is_glyph_atlas_bound_and_atomic() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-render-plan-{id}.md"));
        let source = "# 标题\n\nParagraph **粗体** and 日本語🙂\n\n- [ ] 任务\n\n```rust\nfn main() {}\n```\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );

        let mut snapshot = YuStorageVisualRenderPlanSnapshot::default();
        let mut command_required = 0;
        let mut page_required = 0;
        let mut damage_required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    &mut command_required,
                    &mut page_required,
                    &mut damage_required,
                )
            },
            YU_STORAGE_OK
        );
        assert!(command_required > 0);
        assert!(page_required > 0);
        assert!(damage_required > 0);
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.command_count, command_required as u64);
        assert_eq!(snapshot.upload_count, page_required as u64);
        assert_eq!(snapshot.damage_count, damage_required as u64);
        assert_eq!(snapshot.viewport_width, 500.0);
        assert_eq!(snapshot.embedded_command_count, 0);
        assert_eq!(snapshot.embedded_upload_count, 0);
        assert_eq!(snapshot.embedded_upload_bytes, 0);

        let mut too_small_commands =
            vec![YuStorageVisualRenderCommand::default(); command_required - 1];
        let mut pages = vec![YuStorageVisualRenderPage::default(); page_required];
        let mut damage = vec![YuStorageVisualRenderDamage::default(); damage_required];
        let mut written_commands = 0;
        let mut written_pages = 0;
        let mut written_damage = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    too_small_commands.as_mut_ptr(),
                    too_small_commands.len(),
                    pages.as_mut_ptr(),
                    pages.len(),
                    damage.as_mut_ptr(),
                    damage.len(),
                    &mut written_commands,
                    &mut written_pages,
                    &mut written_damage,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );
        assert_eq!(written_commands, command_required);
        assert_eq!(written_pages, page_required);
        assert_eq!(written_damage, damage_required);
        assert!(
            too_small_commands
                .iter()
                .all(|command| *command == YuStorageVisualRenderCommand::default())
        );
        assert!(
            pages
                .iter()
                .all(|page| *page == YuStorageVisualRenderPage::default())
        );
        assert!(
            damage
                .iter()
                .all(|rect| *rect == YuStorageVisualRenderDamage::default())
        );

        let mut commands = vec![YuStorageVisualRenderCommand::default(); command_required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    commands.as_mut_ptr(),
                    commands.len(),
                    pages.as_mut_ptr(),
                    pages.len(),
                    damage.as_mut_ptr(),
                    damage.len(),
                    &mut written_commands,
                    &mut written_pages,
                    &mut written_damage,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written_commands, command_required);
        assert_eq!(written_pages, page_required);
        assert_eq!(written_damage, damage_required);
        assert!(commands.iter().all(|command| {
            let geometry_valid = command.bounds_width.is_finite()
                && command.bounds_height.is_finite()
                && command.bounds_width >= 0.0
                && command.bounds_height >= 0.0
                && command.origin_x.is_finite()
                && command.origin_y.is_finite();
            command.revision == 0
                && geometry_valid
                && command.embedded_generation == 0
                && command.embedded_kind == 0
                && command.embedded_width == 0
                && command.embedded_height == 0
                && match command.kind {
                    YU_STORAGE_RENDER_COMMAND_FILL_RECT => {
                        command.page == YU_STORAGE_RENDER_PAGE_NONE
                            && command.atlas_width == 0
                            && command.atlas_height == 0
                            && command.advance_x == 0.0
                    }
                    YU_STORAGE_RENDER_COMMAND_GLYPH => command.advance_x.is_finite(),
                    _ => false,
                }
        }));
        assert!(
            commands
                .iter()
                .any(|command| command.kind == YU_STORAGE_RENDER_COMMAND_FILL_RECT)
        );
        assert!(
            commands
                .iter()
                .any(|command| command.kind == YU_STORAGE_RENDER_COMMAND_GLYPH)
        );
        assert!(commands.windows(2).all(|pair| {
            pair[0].block_index <= pair[1].block_index
                && pair[0].source_end_utf16 <= pair[1].source_end_utf16
        }));
        assert!(pages.windows(2).all(|pair| pair[0].page < pair[1].page));
        assert!(pages.iter().all(|page| {
            page.revision == 0 && page.width > 0 && page.height > 0 && page.fingerprint != 0
        }));
        assert!(damage.iter().all(|rect| {
            rect.revision == 0
                && rect.x.is_finite()
                && rect.y.is_finite()
                && rect.width.is_finite()
                && rect.height.is_finite()
                && rect.width >= 0.0
                && rect.height >= 0.0
        }));

        let composition_start = source.find("粗体").expect("composition target");
        let composition_start_utf16 = source[..composition_start].encode_utf16().count() as u64;
        let composition_end_utf16 = composition_start_utf16 + "粗体".encode_utf16().count() as u64;
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
        let mut stale_commands = vec![YuStorageVisualRenderCommand::default(); commands.len()];
        let mut stale_pages = vec![YuStorageVisualRenderPage::default(); pages.len()];
        let mut stale_damage = vec![YuStorageVisualRenderDamage::default(); damage.len()];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    stale_commands.as_mut_ptr(),
                    stale_commands.len(),
                    stale_pages.as_mut_ptr(),
                    stale_pages.len(),
                    stale_damage.as_mut_ptr(),
                    stale_damage.len(),
                    &mut written_commands,
                    &mut written_pages,
                    &mut written_damage,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        assert_eq!(snapshot, YuStorageVisualRenderPlanSnapshot::default());
        assert_eq!(written_commands, 0);
        assert_eq!(written_pages, 0);
        assert_eq!(written_damage, 0);

        let mut composition_snapshot = YuStorageVisualRenderPlanSnapshot::default();
        let mut composition_command_required = 0;
        let mut composition_page_required = 0;
        let mut composition_damage_required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut composition_snapshot,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    &mut composition_command_required,
                    &mut composition_page_required,
                    &mut composition_damage_required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(composition_snapshot.revision, 0);
        assert_eq!(composition_snapshot.composition_generation, 1);
        assert!(composition_command_required > 0);
        assert!(composition_page_required > 0);
        assert!(composition_damage_required > 0);

        let mut composition_commands =
            vec![YuStorageVisualRenderCommand::default(); composition_command_required];
        let mut composition_pages =
            vec![YuStorageVisualRenderPage::default(); composition_page_required];
        let mut composition_damage =
            vec![YuStorageVisualRenderDamage::default(); composition_damage_required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut composition_snapshot,
                    composition_commands.as_mut_ptr(),
                    composition_commands.len(),
                    composition_pages.as_mut_ptr(),
                    composition_pages.len(),
                    composition_damage.as_mut_ptr(),
                    composition_damage.len(),
                    &mut written_commands,
                    &mut written_pages,
                    &mut written_damage,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written_commands, composition_command_required);
        assert_eq!(written_pages, composition_page_required);
        assert_eq!(written_damage, composition_damage_required);
        assert!(composition_commands.iter().all(|command| {
            command.revision == 0
                && command.bounds_width.is_finite()
                && command.bounds_height.is_finite()
        }));

        assert_eq!(
            unsafe {
                yu_storage_session_update_composition(
                    raw,
                    0,
                    1,
                    "日本語".as_ptr(),
                    "日本語".len(),
                    3,
                    3,
                )
            },
            YU_STORAGE_OK
        );
        let mut stale_after_update_commands =
            vec![YuStorageVisualRenderCommand::default(); composition_command_required];
        let mut stale_after_update_pages =
            vec![YuStorageVisualRenderPage::default(); composition_page_required];
        let mut stale_after_update_damage =
            vec![YuStorageVisualRenderDamage::default(); composition_damage_required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut composition_snapshot,
                    stale_after_update_commands.as_mut_ptr(),
                    stale_after_update_commands.len(),
                    stale_after_update_pages.as_mut_ptr(),
                    stale_after_update_pages.len(),
                    stale_after_update_damage.as_mut_ptr(),
                    stale_after_update_damage.len(),
                    &mut written_commands,
                    &mut written_pages,
                    &mut written_damage,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        assert_eq!(
            composition_snapshot,
            YuStorageVisualRenderPlanSnapshot::default()
        );
        assert_eq!(written_commands, 0);
        assert_eq!(written_pages, 0);
        assert_eq!(written_damage, 0);
        assert!(
            stale_after_update_commands
                .iter()
                .all(|command| *command == YuStorageVisualRenderCommand::default())
        );

        let mut updated_composition_snapshot = YuStorageVisualRenderPlanSnapshot::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut updated_composition_snapshot,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    &mut composition_command_required,
                    &mut composition_page_required,
                    &mut composition_damage_required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(updated_composition_snapshot.composition_generation, 2);
        assert!(composition_command_required > 0);

        assert_eq!(
            unsafe { yu_storage_session_cancel_composition(raw, 0, 2) },
            YU_STORAGE_OK
        );
        let mut stale_after_cancel_commands =
            vec![YuStorageVisualRenderCommand::default(); composition_command_required];
        let mut stale_after_cancel_pages =
            vec![YuStorageVisualRenderPage::default(); composition_page_required];
        let mut stale_after_cancel_damage =
            vec![YuStorageVisualRenderDamage::default(); composition_damage_required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut updated_composition_snapshot,
                    stale_after_cancel_commands.as_mut_ptr(),
                    stale_after_cancel_commands.len(),
                    stale_after_cancel_pages.as_mut_ptr(),
                    stale_after_cancel_pages.len(),
                    stale_after_cancel_damage.as_mut_ptr(),
                    stale_after_cancel_damage.len(),
                    &mut written_commands,
                    &mut written_pages,
                    &mut written_damage,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        assert_eq!(
            updated_composition_snapshot,
            YuStorageVisualRenderPlanSnapshot::default()
        );
        assert_eq!(written_commands, 0);
        assert_eq!(written_pages, 0);
        assert_eq!(written_damage, 0);

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_render_plan(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    0,
                    &mut written_commands,
                    &mut written_pages,
                    &mut written_damage,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(snapshot, YuStorageVisualRenderPlanSnapshot::default());
        assert_eq!(written_commands, 0);
        assert_eq!(written_pages, 0);
        assert_eq!(written_damage, 0);

        unsafe { yu_storage_session_destroy(raw) };
        fs::remove_file(path).expect("cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_render_host_config_tracks_document_scroll_origin() {
        let viewport = ViewportRect::new(137.5, 240.0);
        let config = macos_render_host_config(viewport, 14.0, 500.0, 240.0)
            .expect("valid macOS render host config");

        assert_eq!(config.viewport().scroll_y(), 137.5);
        assert_eq!(config.viewport().height(), 240.0);
        assert_eq!(config.scene_viewport().x(), 0.0);
        assert_eq!(config.scene_viewport().y(), 137.5);
        assert_eq!(config.scene_viewport().width(), 500.0);
        assert_eq!(config.scene_viewport().height(), 240.0);
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

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
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
                yu_storage_session_macos_table_resize_hit_test(
                    raw,
                    0,
                    14.0,
                    500.0,
                    divider + 0.01,
                    point_y,
                    0.2,
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
                yu_storage_session_macos_table_resize_begin_at_point(
                    raw,
                    0,
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
            unsafe { yu_storage_session_table_resize_cancel(raw, 0) },
            YU_STORAGE_OK
        );
        let mut hit = YuStorageTableResizeHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_table_resize_begin(
                    raw,
                    0,
                    0,
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
            unsafe { yu_storage_session_table_resize_update(raw, 0, divider + 1.01, &mut preview) },
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
            unsafe { yu_storage_session_table_resize_finish(raw, 0, &mut committed) },
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
                yu_storage_session_macos_table_resize_hit_test(
                    raw,
                    0,
                    14.0,
                    500.0,
                    effective_divider.x + 0.01,
                    point_y,
                    0.2,
                    &mut effective_hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(effective_hit.index, 0);
        assert_eq!(
            unsafe { yu_storage_session_table_resize_cancel(raw, 0) },
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

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
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
        assert_ne!(
            first.command_kind_mask & (1_u64 << u32::from(YU_STORAGE_RENDER_COMMAND_GLYPH)),
            0
        );
        let supported_command_mask = (1_u64 << u32::from(YU_STORAGE_RENDER_COMMAND_FILL_RECT))
            | (1_u64 << u32::from(YU_STORAGE_RENDER_COMMAND_GLYPH))
            | (1_u64 << u32::from(YU_STORAGE_RENDER_COMMAND_IMAGE))
            | (1_u64 << u32::from(YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG));
        assert_eq!(first.command_kind_mask & !supported_command_mask, 0);
        let supported_block_kind_mask = (0..=YU_STORAGE_PROJECTION_BLOCK_TASK_LIST_ITEM)
            .fold(0_u64, |mask, kind| mask | (1_u64 << u32::from(kind)));
        assert_ne!(first.block_kind_mask, 0);
        assert_eq!(first.block_kind_mask & !supported_block_kind_mask, 0);

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
        assert_eq!(cross_block.command_kind_mask, first.command_kind_mask);
        assert_eq!(cross_block.block_kind_mask, first.block_kind_mask);
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
    fn ffi_macos_visual_scene_glyphs_are_retained_and_source_backed() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yu-storage-ffi-macos-glyph-scene-{id}.md"));
        let source = "# 羽🙂\n\nParagraph **粗体** and 日本語\n\n```rust\nfn main() {}\n```\n";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );

        let mut snapshot = YuStorageVisualSceneGlyphSnapshot::default();
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene_glyphs(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert!(required > 0);
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.frame_revision, 0);
        assert_eq!(snapshot.frame_serial, 1);
        assert_eq!(snapshot.glyph_count, required as u64);
        assert!(snapshot.block_end > snapshot.block_start);

        let mut too_small = vec![YuStorageVisualSceneGlyph::default(); required - 1];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene_glyphs(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    0,
                    &mut snapshot,
                    too_small.as_mut_ptr(),
                    too_small.len(),
                    &mut written,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );
        assert_eq!(written, required);
        assert!(
            too_small
                .iter()
                .all(|glyph| *glyph == YuStorageVisualSceneGlyph::default())
        );

        let mut glyphs = vec![YuStorageVisualSceneGlyph::default(); required];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene_glyphs(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    0,
                    &mut snapshot,
                    glyphs.as_mut_ptr(),
                    glyphs.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, required);
        assert!(glyphs.windows(2).all(|pair| {
            pair[0].revision == 0
                && pair[1].revision == 0
                && pair[0].block_index <= pair[1].block_index
                && pair[0].source_start_utf16 <= pair[1].source_start_utf16
        }));
        assert!(glyphs.iter().all(|glyph| {
            glyph.bounds_width.is_finite()
                && glyph.bounds_height.is_finite()
                && glyph.bounds_width >= 0.0
                && glyph.bounds_height >= 0.0
                && glyph.origin_x.is_finite()
                && glyph.origin_y.is_finite()
                && glyph.advance_x.is_finite()
        }));
        assert!(
            glyphs
                .iter()
                .any(|glyph| glyph.page != YU_STORAGE_RENDER_PAGE_NONE)
        );

        let composition_start = source.find("粗体").expect("composition target");
        let composition_start_utf16 = source[..composition_start].encode_utf16().count() as u64;
        let composition_end_utf16 = composition_start_utf16 + "粗体".encode_utf16().count() as u64;
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
        let mut stale_glyphs = vec![YuStorageVisualSceneGlyph::default(); glyphs.len()];
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene_glyphs(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    0,
                    &mut snapshot,
                    stale_glyphs.as_mut_ptr(),
                    stale_glyphs.len(),
                    &mut written,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        assert_eq!(snapshot, YuStorageVisualSceneGlyphSnapshot::default());
        assert_eq!(written, 0);
        assert_eq!(
            unsafe { yu_storage_session_cancel_composition(raw, 0, 1) },
            YU_STORAGE_OK
        );

        let mut result = YuStorageCommandResult::default();
        assert_eq!(
            unsafe { yu_storage_session_insert_text(raw, 0, b"!".as_ptr(), 1, &mut result) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_macos_visual_scene_glyphs(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    0,
                    &mut snapshot,
                    ptr::null_mut(),
                    0,
                    &mut written,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(snapshot, YuStorageVisualSceneGlyphSnapshot::default());
        assert_eq!(written, 0);

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

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );
        let source_utf16 = source.encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_set_selection(
                    raw,
                    0,
                    source_utf16,
                    source_utf16,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                )
            },
            YU_STORAGE_OK
        );

        let viewport_height = metrics.line_height();
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
                    0.0,
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
                    0.0,
                    &mut request,
                )
            },
            YU_STORAGE_STALE_REVISION
        );
        assert_eq!(request, YuStorageCaretScrollRequest::default());

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

        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_copy_composition_projection(
                    raw,
                    projection.revision,
                    projection.generation,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(required, projection.projected_utf8_length as usize);
        let mut small = vec![0_u8; required.saturating_sub(1)];
        assert_eq!(
            unsafe {
                yu_storage_session_copy_composition_projection(
                    raw,
                    projection.revision,
                    projection.generation,
                    small.as_mut_ptr(),
                    small.len(),
                    &mut required,
                )
            },
            YU_STORAGE_BUFFER_TOO_SMALL
        );
        let mut projected = vec![0_u8; required];
        assert_eq!(
            unsafe {
                yu_storage_session_copy_composition_projection(
                    raw,
                    projection.revision,
                    projection.generation,
                    projected.as_mut_ptr(),
                    projected.len(),
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(
            std::str::from_utf8(&projected).expect("projected text stays UTF-8"),
            "before 日本🙂 after"
        );

        let mut caret = YuStorageCompositionCaret::default();
        assert_eq!(
            unsafe {
                yu_storage_session_composition_caret(
                    raw,
                    projection.revision,
                    projection.generation,
                    9,
                    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM,
                    &mut caret,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(caret.revision, 0);
        assert_eq!(caret.generation, 1);
        assert_eq!(caret.source_utf16, 9);
        assert_eq!(caret.visual_utf16, 11);
        assert_eq!(caret.visual_selection_start_utf16, 9);
        assert_eq!(caret.visual_selection_end_utf16, 11);
        let mut invalid_caret = YuStorageCompositionCaret {
            revision: 99,
            ..YuStorageCompositionCaret::default()
        };
        assert_eq!(
            unsafe {
                yu_storage_session_composition_caret(
                    raw,
                    projection.revision,
                    projection.generation,
                    9,
                    99,
                    &mut invalid_caret,
                )
            },
            YU_STORAGE_INVALID_SELECTION
        );
        assert_eq!(invalid_caret, YuStorageCompositionCaret::default());

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
        assert_eq!(
            unsafe {
                yu_storage_session_copy_composition_projection(
                    raw,
                    0,
                    projection.generation,
                    ptr::null_mut(),
                    0,
                    &mut required,
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

        let mut metrics = YuStorageMacosFontMetrics::default();
        assert_eq!(
            unsafe { yu_storage_session_macos_font_metrics(raw, 0, 14.0, 500.0, &mut metrics) },
            YU_STORAGE_OK
        );
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height,
                    metrics.default_advance,
                    metrics.line_height,
                    0.0,
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

    #[cfg(target_os = "macos")]
    #[test]
    fn ffi_macos_composition_hit_test_maps_cross_block_transient_coordinates() {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yu-storage-ffi-composition-hit-test-cross-block-{id}.md"
        ));
        let source = "first **x**\n\nsecond 日本語";
        fs::write(&path, source).expect("fixture");
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        let mut raw = ptr::null_mut();
        assert_eq!(
            unsafe { yu_storage_session_open(path_bytes.as_ptr(), path_bytes.len(), &mut raw) },
            YU_STORAGE_OK
        );
        let replacement_start = source.find('x').expect("replacement start");
        let replacement_end = source.find("日本語").expect("replacement end") + "日本".len();
        let replacement_start_utf16 = source[..replacement_start].encode_utf16().count() as u64;
        let replacement_end_utf16 = source[..replacement_end].encode_utf16().count() as u64;
        assert_eq!(
            unsafe {
                yu_storage_session_begin_composition(
                    raw,
                    0,
                    replacement_start_utf16,
                    replacement_end_utf16,
                    "日本🙂".as_ptr(),
                    "日本🙂".len(),
                    2,
                    2,
                )
            },
            YU_STORAGE_OK
        );

        let (_, metrics, _) = core_text_system_ui_layout(14.0, 500.0).expect("CoreText");
        assert_eq!(
            unsafe {
                yu_storage_session_set_viewport_config(
                    raw,
                    0,
                    500.0,
                    metrics.line_height(),
                    metrics.default_advance(),
                    metrics.line_height(),
                    0.0,
                )
            },
            YU_STORAGE_OK
        );
        let mut viewport = YuStorageShapedViewportSnapshot::default();
        let mut required = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_shaped_viewport_blocks(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut viewport,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            },
            YU_STORAGE_OK
        );
        let mut blocks = vec![YuStorageShapedViewportBlock::default(); required];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_macos_shaped_viewport_blocks(
                    raw,
                    0,
                    14.0,
                    500.0,
                    0.0,
                    1_000.0,
                    &mut viewport,
                    blocks.as_mut_ptr(),
                    blocks.len(),
                    &mut written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(written, blocks.len());
        let second_start = source.find("second").expect("second block");
        let second_start_utf16 = source[..second_start].encode_utf16().count() as u64;
        let second = blocks
            .iter()
            .find(|block| {
                block.source_start_utf16 <= second_start_utf16
                    && block.source_end_utf16 >= second_start_utf16
            })
            .copied()
            .or_else(|| blocks.get(2).copied())
            .expect("second block geometry");

        let mut composition_hit = YuStorageCompositionProjectionHit::default();
        assert_eq!(
            unsafe {
                yu_storage_session_macos_composition_projection_hit_test(
                    raw,
                    0,
                    1,
                    499.0,
                    second.y + second.height * 0.5,
                    14.0,
                    500.0,
                    &mut composition_hit,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(composition_hit.revision, 0);
        assert_eq!(composition_hit.generation, 1);
        assert_eq!(composition_hit.block_index, second.block_index);
        assert!(composition_hit.x.is_finite());
        assert!(composition_hit.y.is_finite());
        assert!(composition_hit.visual_utf16 >= composition_hit.visual_replacement_start_utf16);
        assert!(
            composition_hit.visual_selection_end_utf16
                >= composition_hit.visual_selection_start_utf16
        );

        let mut projection = YuStorageCompositionProjection::default();
        assert_eq!(
            unsafe { yu_storage_session_composition_projection(raw, 0, &mut projection) },
            YU_STORAGE_OK
        );
        assert_eq!(
            composition_hit.visual_selection_start_utf16,
            projection.visual_selection_start_utf16
        );
        assert_eq!(
            composition_hit.visual_selection_end_utf16,
            projection.visual_selection_end_utf16
        );
        assert_eq!(
            composition_hit.visual_replacement_start_utf16,
            projection.visual_replacement_start_utf16
        );
        assert_eq!(
            composition_hit.visual_replacement_end_utf16,
            projection.visual_replacement_end_utf16
        );

        let mut stale = YuStorageCompositionProjectionHit {
            revision: 99,
            ..YuStorageCompositionProjectionHit::default()
        };
        assert_eq!(
            unsafe {
                yu_storage_session_macos_composition_projection_hit_test(
                    raw,
                    0,
                    2,
                    499.0,
                    second.y + second.height * 0.5,
                    14.0,
                    500.0,
                    &mut stale,
                )
            },
            YU_STORAGE_STALE_COMPOSITION
        );
        assert_eq!(stale, YuStorageCompositionProjectionHit::default());

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
        assert_eq!(snapshot.selection_start_utf16, 11);
        assert_eq!(snapshot.selection_end_utf16, 11);
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
            unsafe { yu_storage_session_accessibility_semantic_node_count(raw, 0, &mut count) },
            YU_STORAGE_OK
        );
        assert!(
            count >= 6,
            "root, blocks, and inline semantic nodes (count={count})"
        );

        let mut nodes = vec![YuStorageAccessibilityNode::default(); count];
        let mut written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_semantic_nodes(
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
        let mut extended_nodes = vec![YuStorageAccessibilityNodeV2::default(); count];
        let mut extended_written = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_semantic_nodes_v2(
                    raw,
                    0,
                    extended_nodes.as_mut_ptr(),
                    extended_nodes.len(),
                    &mut extended_written,
                )
            },
            YU_STORAGE_OK
        );
        assert_eq!(extended_written, count);
        let link = extended_nodes
            .iter()
            .find(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_LINK)
            .expect("link semantic node");
        assert!(link.destination_end_utf16 > link.destination_start_utf16);
        let reference_link = extended_nodes
            .iter()
            .find(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_REFERENCE_LINK)
            .expect("reference link semantic node");
        assert!(reference_link.destination_end_utf16 > reference_link.destination_start_utf16);
        let task = extended_nodes
            .iter()
            .find(|node| node.kind == YU_STORAGE_ACCESSIBILITY_KIND_TASK_LIST_ITEM)
            .expect("task semantic node");
        assert_ne!(task.flags & YU_STORAGE_ACCESSIBILITY_FLAG_TASK_DONE, 0);
        assert_ne!(task.action_block, YU_STORAGE_ACCESSIBILITY_NO_ACTION_BLOCK);

        let mut stale_count = 0;
        assert_eq!(
            unsafe {
                yu_storage_session_accessibility_semantic_node_count(raw, 1, &mut stale_count)
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

        let mut selection = YuStorageSelection::default();
        assert_eq!(
            unsafe { yu_storage_session_selection(raw, &mut selection) },
            YU_STORAGE_OK
        );
        assert_eq!(selection.revision, 0);
        assert_eq!(selection.start_utf16, 4);

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
                yu_storage_session_set_selection(raw, 0, 1, 6, YU_STORAGE_CARET_AFFINITY_DOWNSTREAM)
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
                yu_storage_session_set_selection(raw, 0, 0, 5, YU_STORAGE_CARET_AFFINITY_DOWNSTREAM)
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
