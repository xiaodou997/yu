#ifndef YU_STORAGE_FFI_H
#define YU_STORAGE_FFI_H

#include <stddef.h>
#include <stdint.h>

enum {
    YU_STORAGE_OK = 0,
    YU_STORAGE_NULL_POINTER = 1,
    YU_STORAGE_INVALID_UTF8 = 2,
    YU_STORAGE_IO_ERROR = 3,
    YU_STORAGE_EXTERNAL_CHANGE = 4,
    YU_STORAGE_UNSAVED_CHANGES = 5,
    YU_STORAGE_INVALID_PATH = 6,
    YU_STORAGE_EDITOR_ERROR = 7,
    YU_STORAGE_BUFFER_TOO_SMALL = 8,
    YU_STORAGE_INVALID_STATE = 9,
    YU_STORAGE_KEY_UNHANDLED = 10,
    YU_STORAGE_INVALID_COMMAND = 11,
    YU_STORAGE_INVALID_KEY = 12,
    YU_STORAGE_STALE_REVISION = 13,
    YU_STORAGE_INVALID_SELECTION = 14,
    YU_STORAGE_NO_OVERLAY = 15,
    YU_STORAGE_STALE_COMPOSITION = 16,
    YU_STORAGE_EXPORT_ERROR = 17,
    YU_STORAGE_HTML_IMPORT_REJECTED = 18,
    YU_STORAGE_CORE_TEXT_UNAVAILABLE = 19,
    YU_STORAGE_INVALID_VIEWPORT_CONFIG = 20,
    YU_STORAGE_RENDER_HOST_UNAVAILABLE = 21,
};

enum {
    YU_STORAGE_SCENE_PRIMITIVE_BACKGROUND = 0,
    YU_STORAGE_SCENE_PRIMITIVE_TEXT_BOUNDS = 1,
};

enum {
    YU_STORAGE_KEY_CHARACTER = 0,
    YU_STORAGE_KEY_ENTER = 1,
    YU_STORAGE_KEY_TAB = 2,
    YU_STORAGE_KEY_BACKSPACE = 3,
    YU_STORAGE_KEY_DELETE = 4,
    YU_STORAGE_KEY_LEFT = 5,
    YU_STORAGE_KEY_RIGHT = 6,
    YU_STORAGE_KEY_UP = 7,
    YU_STORAGE_KEY_DOWN = 8,
    YU_STORAGE_KEY_ESCAPE = 9,
};

enum {
    YU_STORAGE_KEY_MODIFIER_COMMAND = 1 << 0,
    YU_STORAGE_KEY_MODIFIER_SHIFT = 1 << 1,
    YU_STORAGE_KEY_MODIFIER_CONTROL = 1 << 2,
    YU_STORAGE_KEY_MODIFIER_OPTION = 1 << 3,
};

enum {
    YU_STORAGE_COMMAND_DELETE_BACKWARD = 1,
    YU_STORAGE_COMMAND_DELETE_FORWARD = 2,
    YU_STORAGE_COMMAND_MOVE_LEFT = 3,
    YU_STORAGE_COMMAND_MOVE_RIGHT = 4,
    YU_STORAGE_COMMAND_INSERT_NEWLINE = 5,
    YU_STORAGE_COMMAND_INDENT_LIST = 6,
    YU_STORAGE_COMMAND_OUTDENT_LIST = 7,
    YU_STORAGE_COMMAND_UNDO = 8,
    YU_STORAGE_COMMAND_REDO = 9,
    YU_STORAGE_COMMAND_TOGGLE_TASK = 10,
    YU_STORAGE_COMMAND_MOVE_WORD_LEFT = 11,
    YU_STORAGE_COMMAND_MOVE_WORD_RIGHT = 12,
    YU_STORAGE_COMMAND_MOVE_UP = 13,
    YU_STORAGE_COMMAND_MOVE_DOWN = 14,
    YU_STORAGE_COMMAND_MOVE_UP_EXTEND = 15,
    YU_STORAGE_COMMAND_MOVE_DOWN_EXTEND = 16,
};

enum {
    YU_STORAGE_SOURCE_SYNC_NONE = 0,
    YU_STORAGE_SOURCE_SYNC_RANGE = 1,
    YU_STORAGE_SOURCE_SYNC_FULL = 2,
    YU_STORAGE_CARET_AFFINITY_UPSTREAM = 0,
    YU_STORAGE_CARET_AFFINITY_DOWNSTREAM = 1,
};

/* Stable parser-owned block tags used by projection snapshots. These values
 * mirror BlockKind::viewport_tag(), but the Rust enum layout is not part of
 * the C ABI. */
enum {
    YU_STORAGE_PROJECTION_BLOCK_BLANK_LINE = 0,
    YU_STORAGE_PROJECTION_BLOCK_REFERENCE_DEFINITION = 1,
    YU_STORAGE_PROJECTION_BLOCK_PARAGRAPH = 2,
    YU_STORAGE_PROJECTION_BLOCK_HEADING = 3,
    YU_STORAGE_PROJECTION_BLOCK_FENCED_CODE = 4,
    YU_STORAGE_PROJECTION_BLOCK_BLOCK_QUOTE = 5,
    YU_STORAGE_PROJECTION_BLOCK_LIST_ITEM = 6,
    YU_STORAGE_PROJECTION_BLOCK_TASK_LIST_ITEM = 7,
};

enum {
    YU_STORAGE_PROJECTION_INLINE = 0,
    YU_STORAGE_PROJECTION_FENCED_CODE = 1,
    YU_STORAGE_PROJECTION_REFERENCE_DEFINITION = 2,
    YU_STORAGE_PROJECTION_TASK_LIST = 3,
    YU_STORAGE_PROJECTION_HEADING = 4,
    YU_STORAGE_PROJECTION_BLOCK_QUOTE = 5,
    YU_STORAGE_PROJECTION_LIST = 6,
    YU_STORAGE_PROJECTION_TABLE = 7,
};

enum {
    YU_STORAGE_TABLE_ALIGNMENT_DEFAULT = 0,
    YU_STORAGE_TABLE_ALIGNMENT_LEFT = 1,
    YU_STORAGE_TABLE_ALIGNMENT_CENTER = 2,
    YU_STORAGE_TABLE_ALIGNMENT_RIGHT = 3,
};

enum {
    YU_STORAGE_TABLE_RESIZE_NONE = 0,
    YU_STORAGE_TABLE_RESIZE_COLUMN = 1,
    YU_STORAGE_TABLE_RESIZE_ROW = 2,
};

enum {
    YU_STORAGE_DISK_UNCHANGED = 0,
    YU_STORAGE_DISK_CHANGED = 1,
    YU_STORAGE_DISK_MISSING = 2,
};

enum {
    YU_STORAGE_BOM_ABSENT = 0,
    YU_STORAGE_BOM_PRESENT = 1,
};

enum {
    YU_STORAGE_CLOSE_OPEN = 0,
    YU_STORAGE_CLOSE_CLOSED = 1,
    YU_STORAGE_CLOSE_PROMPT_SAVE = 2,
    YU_STORAGE_CLOSE_PROMPT_EXTERNAL_CHANGED = 3,
    YU_STORAGE_CLOSE_PROMPT_EXTERNAL_MISSING = 4,
};

enum {
    YU_STORAGE_CLOSE_NOW = 0,
    YU_STORAGE_CLOSE_PROMPT = 1,
    YU_STORAGE_CLOSE_ALREADY_CLOSED = 2,
};

enum {
    YU_STORAGE_ACCESSIBILITY_PARENT_NONE = UINT32_MAX,
    YU_STORAGE_ACCESSIBILITY_FLAG_ORDERED = 1 << 0,
    YU_STORAGE_ACCESSIBILITY_FLAG_TASK_DONE = 1 << 1,
    YU_STORAGE_ACCESSIBILITY_KIND_DOCUMENT = 1,
    YU_STORAGE_ACCESSIBILITY_KIND_HEADING = 2,
    YU_STORAGE_ACCESSIBILITY_KIND_PARAGRAPH = 3,
    YU_STORAGE_ACCESSIBILITY_KIND_CODE_BLOCK = 4,
    YU_STORAGE_ACCESSIBILITY_KIND_BLOCK_QUOTE = 5,
    YU_STORAGE_ACCESSIBILITY_KIND_LIST_ITEM = 6,
    YU_STORAGE_ACCESSIBILITY_KIND_TASK_LIST_ITEM = 7,
    YU_STORAGE_ACCESSIBILITY_KIND_EMPHASIS = 8,
    YU_STORAGE_ACCESSIBILITY_KIND_STRONG = 9,
    YU_STORAGE_ACCESSIBILITY_KIND_CODE_SPAN = 10,
    YU_STORAGE_ACCESSIBILITY_KIND_LINK = 11,
    YU_STORAGE_ACCESSIBILITY_KIND_IMAGE = 12,
    YU_STORAGE_ACCESSIBILITY_KIND_AUTOLINK = 13,
    YU_STORAGE_ACCESSIBILITY_KIND_REFERENCE_LINK = 14,
    YU_STORAGE_ACCESSIBILITY_KIND_REFERENCE_IMAGE = 15,
};

#define YU_STORAGE_ACCESSIBILITY_NO_RANGE UINT64_MAX
#define YU_STORAGE_ACCESSIBILITY_NO_ACTION_BLOCK UINT64_MAX

typedef struct YuStorageSession YuStorageSession;

typedef struct YuStorageState {
    uint64_t revision;
    uint64_t saved_revision;
    uint8_t dirty;
    uint8_t disk_state;
    uint8_t bom;
    uint8_t close_state;
} YuStorageState;

typedef struct YuStorageCloseRequest {
    uint8_t result;
    uint8_t close_state;
} YuStorageCloseRequest;

typedef struct YuStorageSelection {
    uint64_t revision;
    uint64_t start_utf16;
    uint64_t end_utf16;
    uint8_t affinity;
} YuStorageSelection;

/* Revision-bound selection endpoints. The anchor/focus pair preserves the
 * direction of a native visual drag; YuStorageSelection remains the ordered
 * range used by legacy callers. */
typedef struct YuStorageSelectionEndpoints {
    uint64_t revision;
    uint64_t anchor_utf16;
    uint64_t focus_utf16;
    uint8_t affinity;
} YuStorageSelectionEndpoints;

typedef struct YuStorageProjectionCaret {
    uint64_t revision;
    uint64_t source_utf16;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
    uint8_t affinity;
} YuStorageProjectionCaret;

/* Revision-bound source selection projected into visual UTF-16 coordinates.
 * Non-collapsed source boundaries use the outer projection edges, so hidden
 * Markdown delimiters are not reintroduced into the visual selection. */
typedef struct YuStorageProjectionSelection {
    uint64_t revision;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t visual_start_utf16;
    uint64_t visual_end_utf16;
    uint64_t round_trip_source_start_utf16;
    uint64_t round_trip_source_end_utf16;
    uint8_t affinity;
} YuStorageProjectionSelection;

/* Reverse caret mapping for a native visual mirror. All offsets are UTF-16. */
typedef struct YuStorageProjectionSourceCaret {
    uint64_t revision;
    uint64_t visual_utf16;
    uint64_t source_utf16;
    uint64_t round_trip_visual_utf16;
    uint8_t affinity;
} YuStorageProjectionSourceCaret;

/* Reverse selection mapping for a native visual mirror. Non-collapsed visual
 * edges map to the outer source range, retaining hidden Markdown syntax. */
typedef struct YuStorageProjectionSourceSelection {
    uint64_t revision;
    uint64_t visual_start_utf16;
    uint64_t visual_end_utf16;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t round_trip_visual_start_utf16;
    uint64_t round_trip_visual_end_utf16;
    uint8_t affinity;
} YuStorageProjectionSourceSelection;

/* Revision-bound metrics-layout hit-test result. x/y are projection-local
 * layout coordinates, not screen coordinates. */
typedef struct YuStorageProjectionHit {
    uint64_t revision;
    uint64_t source_utf16;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
    /* Complete Markdown image source range, or
     * YU_STORAGE_IMAGE_DESTINATION_NONE for a regular text hit. */
    uint64_t image_source_start_utf16;
    uint64_t image_source_end_utf16;
    uint64_t line;
    float x;
    float y;
    uint8_t affinity;
} YuStorageProjectionHit;

/* Revision/generation-bound transient source/visual projection metadata.
 * Canonical source remains unchanged while a composition is active. */
typedef struct YuStorageCompositionProjection {
    uint64_t revision;
    uint64_t generation;
    uint64_t replacement_start_utf16;
    uint64_t replacement_end_utf16;
    uint64_t preedit_selection_start_utf16;
    uint64_t preedit_selection_end_utf16;
    uint64_t visual_selection_start_utf16;
    uint64_t visual_selection_end_utf16;
    uint64_t projected_utf16_length;
    uint64_t projected_utf8_length;
    /* UTF-16 range occupied by transient preedit in the visual projection. */
    uint64_t visual_replacement_start_utf16;
    uint64_t visual_replacement_end_utf16;
} YuStorageCompositionProjection;

/* Active marked-text caret in the transient projected stream. The visual
 * selection is returned in projected UTF-16 coordinates. */
typedef struct YuStorageCompositionCaret {
    uint64_t revision;
    uint64_t generation;
    uint64_t source_utf16;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
    uint64_t visual_selection_start_utf16;
    uint64_t visual_selection_end_utf16;
    uint8_t affinity;
} YuStorageCompositionCaret;

/* Revision/generation-bound CoreText-shaped caret geometry for the active
 * marked-text projection. caret_x/caret_y are local to block_index; visual
 * UTF-16 ranges remain in the full transient projected stream. */
typedef struct YuStorageCompositionShapedCaret {
    uint64_t revision;
    uint64_t generation;
    uint64_t source_utf16;
    uint64_t block_index;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
    uint64_t line_index;
    float caret_x;
    float caret_y;
    float caret_width;
    float caret_height;
    uint64_t visual_selection_start_utf16;
    uint64_t visual_selection_end_utf16;
    uint64_t visual_replacement_start_utf16;
    uint64_t visual_replacement_end_utf16;
    uint8_t affinity;
} YuStorageCompositionShapedCaret;

/* Revision/generation-bound CoreText-shaped point hit-test for the active
 * marked-text projection. x/y are document-space coordinates; visual ranges
 * use UTF-16 offsets in the transient projected stream. */
typedef struct YuStorageCompositionProjectionHit {
    uint64_t revision;
    uint64_t generation;
    uint64_t source_utf16;
    uint64_t block_index;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
    uint64_t line;
    float x;
    float y;
    uint64_t visual_selection_start_utf16;
    uint64_t visual_selection_end_utf16;
    uint64_t visual_replacement_start_utf16;
    uint64_t visual_replacement_end_utf16;
    uint8_t affinity;
} YuStorageCompositionProjectionHit;

typedef struct YuStorageProjectionBlock {
    uint64_t revision;
    uint64_t block_index;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t visual_utf8_length;
    uint64_t visual_utf16_length;
    uint8_t kind;
    uint8_t projection_kind;
} YuStorageProjectionBlock;

/* Parser-owned GFM table cell range. row 0 is the header, row 1 is the
 * delimiter, and body rows start at row 2. Offsets are UTF-16 positions in
 * the requested revision. */
typedef struct YuStorageTableCellRange {
    uint64_t row;
    uint64_t column;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
} YuStorageTableCellRange;

/* Revision-bound visible table cell geometry. row 0 is the header and body
 * rows start at row 1; the Markdown delimiter row is source-backed but not
 * present in this visible geometry list. */
typedef struct YuStorageTableLayoutCell {
    uint64_t revision;
    uint64_t block_index;
    uint64_t row;
    uint64_t column;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    float x;
    float y;
    float width;
    float height;
    uint8_t alignment;
} YuStorageTableLayoutCell;

/* Revision-bound hit-test result for one visible table cell. */
typedef struct YuStorageTableCellHit {
    uint64_t revision;
    uint64_t block_index;
    uint64_t row;
    uint64_t column;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    float x;
    float y;
    float width;
    float height;
} YuStorageTableCellHit;

/* Revision-bound hit-test result for an internal visible table divider. kind
 * is YU_STORAGE_TABLE_RESIZE_COLUMN or YU_STORAGE_TABLE_RESIZE_ROW; index is
 * the column/row immediately before the divider and position is local x/y. */
typedef struct YuStorageTableResizeHit {
    uint64_t revision;
    uint64_t block_index;
    uint8_t kind;
    uint64_t index;
    float position;
} YuStorageTableResizeHit;

/* Revision-bound layout metadata for one parser-owned block. width/height
 * are block-local layout points; shaped is non-zero for CoreText output. */
typedef struct YuStorageBlockLayout {
    uint64_t revision;
    uint64_t block_index;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t visual_utf16_length;
    uint64_t line_count;
    float width;
    float height;
    float line_height;
    float default_advance;
    uint8_t kind;
    uint8_t projection_kind;
    uint8_t shaped;
} YuStorageBlockLayout;

/* Revision-bound CoreText metrics used to configure an empty or non-empty
 * viewport before requesting a render-host frame. */
typedef struct YuStorageMacosFontMetrics {
    uint64_t revision;
    float size;
    float line_height;
    float default_advance;
} YuStorageMacosFontMetrics;

/* Revision-bound source caret resolved through one block-local layout. */
typedef struct YuStorageBlockCaret {
    uint64_t revision;
    uint64_t source_utf16;
    uint64_t block_index;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
    uint64_t line_index;
    float caret_x;
    float caret_y;
    float caret_width;
    float caret_height;
    uint8_t affinity;
    uint8_t shaped;
} YuStorageBlockCaret;

/* Revision-bound block metadata returned by a CoreText-shaped viewport query.
 * y/height are document-space point coordinates; source ranges are UTF-16. */
typedef struct YuStorageShapedViewportBlock {
    uint64_t revision;
    uint64_t block_index;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    float y;
    float height;
    uint8_t measured;
    uint8_t kind;
} YuStorageShapedViewportBlock;

typedef struct YuStorageShapedViewportSnapshot {
    uint64_t revision;
    uint64_t block_start;
    uint64_t block_end;
    float content_height;
    /* Native viewport inputs used to interpret document-space block y. */
    float scroll_y;
    float viewport_height;
    float max_scroll_y;
} YuStorageShapedViewportSnapshot;

/* Rust/CoreText-shaped visual decoration geometry. Selection rectangles and
 * caret coordinates are document-space and must be offset by scroll_y before
 * a native viewport sibling paints them. Active marked text returns
 * YU_STORAGE_NO_OVERLAY so the host can retain its transient TextKit path. */
typedef struct YuStorageMacosVisualDecorationSnapshot {
    uint64_t revision;
    uint64_t composition_generation;
    uint64_t selection_count;
    uint8_t caret_present;
    float content_height;
    float scroll_y;
    float viewport_height;
    float max_scroll_y;
    float viewport_width;
} YuStorageMacosVisualDecorationSnapshot;

typedef struct YuStorageMacosVisualDecorationRect {
    uint64_t revision;
    uint64_t block_index;
    uint64_t line_index;
    float x;
    float y;
    float width;
    float height;
    uint8_t kind;
} YuStorageMacosVisualDecorationRect;

typedef struct YuStorageMacosVisualDecorationCaret {
    uint64_t revision;
    uint64_t block_index;
    uint64_t line_index;
    float x;
    float y;
    float width;
    float height;
    uint8_t affinity;
    uint8_t present;
} YuStorageMacosVisualDecorationCaret;

/* Revision-bound shaped caret geometry and absolute document scroll target. */
typedef struct YuStorageCaretScrollRequest {
    uint64_t revision;
    uint64_t source_utf16;
    uint64_t block_index;
    float caret_x;
    float caret_y;
    float caret_width;
    float caret_height;
    float current_scroll_y;
    float target_scroll_y;
    float margin;
    uint8_t needs_scroll;
} YuStorageCaretScrollRequest;

/* Revision-bound owned scene metadata assembled by Rust's retained scene
 * boundary. Primitive payloads are currently validated rectangle scalars. */
typedef struct YuStorageVisualSceneSnapshot {
    uint64_t revision;
    uint64_t block_start;
    uint64_t block_end;
    uint64_t primitive_count;
    float content_height;
    float scroll_y;
    float viewport_height;
    float max_scroll_y;
    float viewport_width;
} YuStorageVisualSceneSnapshot;

typedef struct YuStorageVisualScenePrimitive {
    uint64_t revision;
    uint64_t block_index;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    float x;
    float y;
    float width;
    float height;
    uint8_t kind;
} YuStorageVisualScenePrimitive;

enum {
    YU_STORAGE_RENDER_COMMAND_FILL_RECT = 0,
    YU_STORAGE_RENDER_COMMAND_GLYPH = 1,
    YU_STORAGE_RENDER_COMMAND_IMAGE = 2,
    YU_STORAGE_RENDER_PAGE_NONE = UINT32_MAX,
    YU_STORAGE_IMAGE_DESTINATION_NONE = UINT64_MAX,
    YU_STORAGE_IMAGE_INLINE = 0,
    YU_STORAGE_IMAGE_REFERENCE = 1,
};

/* Source-backed image metadata. Destination/reference values are UTF-16
 * ranges in the same Revision; decoded pixels and native texture handles do
 * not cross this ABI. */
typedef struct YuStorageVisualImage {
    uint64_t revision;
    uint64_t block_index;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t label_start_utf16;
    uint64_t label_end_utf16;
    uint64_t destination_start_utf16;
    uint64_t destination_end_utf16;
    uint64_t reference_start_utf16;
    uint64_t reference_end_utf16;
    uint64_t resource_fingerprint;
    uint8_t kind;
} YuStorageVisualImage;

typedef struct YuStorageVisualRenderPlanSnapshot {
    uint64_t revision;
    uint64_t composition_generation;
    uint64_t block_start;
    uint64_t block_end;
    uint64_t command_count;
    uint64_t upload_count;
    uint64_t damage_count;
    float content_height;
    float scroll_y;
    float viewport_height;
    float max_scroll_y;
    float viewport_width;
} YuStorageVisualRenderPlanSnapshot;

typedef struct YuStorageVisualRenderCommand {
    uint64_t revision;
    uint64_t block_index;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint8_t kind;
    uint32_t page;
    uint32_t atlas_x;
    uint32_t atlas_y;
    uint32_t atlas_width;
    uint32_t atlas_height;
    float origin_x;
    float origin_y;
    float bearing_x;
    float bearing_y;
    float advance_x;
    float bounds_x;
    float bounds_y;
    float bounds_width;
    float bounds_height;
    uint32_t color_rgba;
    uint64_t resource;
} YuStorageVisualRenderCommand;

typedef struct YuStorageVisualRenderPage {
    uint64_t revision;
    uint32_t page;
    uint32_t width;
    uint32_t height;
    uint64_t fingerprint;
} YuStorageVisualRenderPage;

typedef struct YuStorageVisualRenderDamage {
    uint64_t revision;
    float x;
    float y;
    float width;
    float height;
} YuStorageVisualRenderDamage;

/* Scalar lifecycle state owned by the persistent Rust macOS render host.
 * UINT64_MAX in frame_revision/frame_serial means that no frame has been
 * accepted for the current Revision yet. Render commands and atlas bytes
 * remain Rust-owned; this snapshot is for native lifecycle coordination. */
typedef struct YuStorageMacosRenderHostSnapshot {
    uint64_t revision;
    uint64_t composition_generation;
    uint64_t frame_revision;
    uint64_t surface_generation;
    uint64_t frame_serial;
    uint64_t command_count;
    uint64_t upload_count;
    uint64_t damage_count;
    uint64_t atlas_page_count;
    uint64_t atlas_glyph_count;
    uint64_t atlas_bytes;
    float content_height;
    float scroll_y;
    float viewport_height;
    float max_scroll_y;
    float viewport_width;
    uint8_t published;
} YuStorageMacosRenderHostSnapshot;

typedef struct YuStorageMacosRenderHostSurfaceSnapshot {
    uint64_t revision;
    uint64_t composition_generation;
    uint64_t surface_generation;
    uint64_t frame_serial;
    uint64_t uploaded_pages;
    uint64_t uploaded_images;
    uint64_t command_count;
    uint64_t damage_count;
    uint64_t atlas_page_count;
    uint64_t image_resource_count;
    uint64_t image_request_count;
    uint64_t image_failure_count;
    uint64_t image_eviction_count;
    uint64_t image_atlas_eviction_count;
    uint64_t image_candidate_count;
    uint64_t image_duplicate_count;
    uint64_t image_visible_candidate_count;
    uint64_t image_overscan_candidate_count;
    uint64_t image_retry_count;
    uint8_t submitted;
} YuStorageMacosRenderHostSurfaceSnapshot;

typedef struct YuStorageVisualSceneGlyph {
    uint64_t revision;
    uint64_t block_index;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint32_t page;
    uint32_t atlas_x;
    uint32_t atlas_y;
    uint32_t atlas_width;
    uint32_t atlas_height;
    float origin_x;
    float origin_y;
    float bearing_x;
    float bearing_y;
    float advance_x;
    float bounds_x;
    float bounds_y;
    float bounds_width;
    float bounds_height;
    uint32_t color_rgba;
} YuStorageVisualSceneGlyph;

typedef struct YuStorageVisualSceneGlyphSnapshot {
    uint64_t revision;
    uint64_t composition_generation;
    uint64_t frame_revision;
    uint64_t surface_generation;
    uint64_t frame_serial;
    uint64_t block_start;
    uint64_t block_end;
    uint64_t glyph_count;
    float content_height;
    float scroll_y;
    float viewport_height;
    float max_scroll_y;
    float viewport_width;
} YuStorageVisualSceneGlyphSnapshot;

typedef struct YuStorageAccessibilitySnapshot {
    uint64_t revision;
    uint64_t number_of_characters_utf16;
    uint64_t selection_start_utf16;
    uint64_t selection_end_utf16;
    uint64_t line_count;
    uint8_t selection_affinity;
} YuStorageAccessibilitySnapshot;

typedef struct YuStorageAccessibilityRange {
    uint64_t revision;
    uint64_t start_utf16;
    uint64_t end_utf16;
} YuStorageAccessibilityRange;

typedef struct YuStorageAccessibilityNode {
    uint64_t revision;
    uint32_t index;
    uint32_t parent;
    uint8_t kind;
    uint8_t flags;
    uint8_t level;
    uint8_t reserved;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t label_start_utf16;
    uint64_t label_end_utf16;
} YuStorageAccessibilityNode;

typedef struct YuStorageAccessibilityNodeV2 {
    uint64_t revision;
    uint32_t index;
    uint32_t parent;
    uint8_t kind;
    uint8_t flags;
    uint8_t level;
    uint8_t reserved;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t label_start_utf16;
    uint64_t label_end_utf16;
    uint64_t destination_start_utf16;
    uint64_t destination_end_utf16;
    uint64_t action_block;
} YuStorageAccessibilityNodeV2;

typedef struct YuStorageCommandResult {
    uint64_t revision;
    uint64_t selection_start_utf16;
    uint64_t selection_end_utf16;
    uint8_t affinity;
    uint8_t changed;
    uint8_t source_sync;
    uint64_t source_start_utf16;
    uint64_t source_old_end_utf16;
    uint64_t source_new_start_utf16;
    uint64_t source_new_end_utf16;
} YuStorageCommandResult;

typedef struct YuStorageCompositionState {
    uint64_t revision;
    uint64_t generation;
    uint64_t replacement_start_utf16;
    uint64_t replacement_end_utf16;
    uint64_t selection_start_utf16;
    uint64_t selection_end_utf16;
    uint64_t preedit_utf8_length;
    uint8_t active;
} YuStorageCompositionState;

int32_t yu_storage_session_open(const uint8_t *path, size_t path_length,
                                YuStorageSession **output);
void yu_storage_session_destroy(YuStorageSession *session);

int32_t yu_storage_session_path_length(const YuStorageSession *session,
                                       size_t *output);
int32_t yu_storage_session_copy_path(const YuStorageSession *session,
                                     uint8_t *output, size_t capacity,
                                     size_t *written);
int32_t yu_storage_session_source_length(const YuStorageSession *session,
                                         size_t *output);
int32_t yu_storage_session_copy_source(const YuStorageSession *session,
                                       uint8_t *output, size_t capacity,
                                       size_t *written);
/* Returns a revision-bound source projection for native layout probes. The
 * result is visual text only; canonical Markdown source remains owned by the
 * session and is queried through yu_storage_session_copy_source. */
int32_t yu_storage_session_projected_source(YuStorageSession *session,
                                            uint64_t expected_revision,
                                            uint8_t *output, size_t capacity,
                                            size_t *written);
int32_t yu_storage_session_projection_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t source_utf16, uint8_t affinity,
    YuStorageProjectionCaret *output);
int32_t yu_storage_session_projection_selection(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t source_start_utf16, uint64_t source_end_utf16,
    uint8_t affinity, YuStorageProjectionSelection *output);
int32_t yu_storage_session_projection_source_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t visual_utf16, uint8_t affinity,
    YuStorageProjectionSourceCaret *output);
int32_t yu_storage_session_projection_source_selection(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t visual_start_utf16, uint64_t visual_end_utf16,
    uint8_t affinity, YuStorageProjectionSourceSelection *output);
int32_t yu_storage_session_projection_hit_test(
    YuStorageSession *session, uint64_t expected_revision,
    float point_x, float point_y, float max_width, float line_height,
    float default_advance, YuStorageProjectionHit *output);
/* Revision-bound macOS CoreText-shaped point hit-test. point_x/point_y are
 * document-space coordinates; returned x/y are snapped document-space caret
 * coordinates. The native TextKit mirror is not consulted. */
int32_t yu_storage_session_macos_projection_hit_test(
    YuStorageSession *session, uint64_t expected_revision,
    float point_x, float point_y, float size, float max_width,
    YuStorageProjectionHit *output);
int32_t yu_storage_session_composition_projection(
    YuStorageSession *session, uint64_t expected_revision,
    YuStorageCompositionProjection *output);
int32_t yu_storage_session_copy_composition_projection(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, uint8_t *output, size_t capacity,
    size_t *written);
int32_t yu_storage_session_composition_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, uint64_t source_utf16, uint8_t affinity,
    YuStorageCompositionCaret *output);
int32_t yu_storage_session_macos_composition_shaped_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, uint64_t source_utf16, uint8_t affinity,
    float size, float max_width, YuStorageCompositionShapedCaret *output);
int32_t yu_storage_session_macos_composition_projection_hit_test(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, float point_x, float point_y,
    float size, float max_width, YuStorageCompositionProjectionHit *output);
int32_t yu_storage_session_projection_block_count(
    const YuStorageSession *session, uint64_t expected_revision,
    size_t *output);
/* Returns one parser-owned block's projected UTF-8 and metadata. A null
 * output with zero capacity is the length-query form; metadata is still
 * filled so the caller can validate source range, kind and visual lengths
 * before allocating its second-call buffer. */
int32_t yu_storage_session_projected_block(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, YuStorageProjectionBlock *metadata,
    uint8_t *output, size_t capacity, size_t *written);
int32_t yu_storage_session_projected_table_cells(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, YuStorageTableCellRange *output,
    size_t capacity, size_t *written);
int32_t yu_storage_session_table_layout_cells(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, float max_width, float line_height,
    float default_advance, YuStorageTableLayoutCell *output,
    size_t capacity, size_t *written);
/* Returns one-call, session-only column geometry. The source and canonical
 * layout remain unchanged; row resize is rejected until variable-row layout
 * exists. */
int32_t yu_storage_session_table_layout_cells_with_resize(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, float max_width, float line_height,
    float default_advance, uint8_t resize_kind, uint64_t resize_index,
    float resize_delta, YuStorageTableLayoutCell *output,
    size_t capacity, size_t *written);
int32_t yu_storage_session_table_cell_hit_test(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, float max_width, float line_height,
    float default_advance, float point_x, float point_y,
    YuStorageTableCellHit *output);
int32_t yu_storage_session_table_resize_hit_test(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, float max_width, float line_height,
    float default_advance, float point_x, float point_y, float tolerance,
    YuStorageTableResizeHit *output);
int32_t yu_storage_session_block_layout(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, float max_width, float line_height,
    float default_advance, YuStorageBlockLayout *output);
int32_t yu_storage_session_macos_block_layout(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, float size, float max_width,
    YuStorageBlockLayout *output);
int32_t yu_storage_session_macos_font_metrics(
    YuStorageSession *session, uint64_t expected_revision,
    float size, float max_width, YuStorageMacosFontMetrics *output);
int32_t yu_storage_session_macos_block_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t block_index, uint64_t source_utf16, uint8_t affinity,
    float size, float max_width, YuStorageBlockCaret *output);
int32_t yu_storage_session_set_viewport_config(
    YuStorageSession *session, uint64_t expected_revision,
    float max_width, float line_height, float default_advance,
    float estimated_block_height, float overscan);
int32_t yu_storage_session_macos_shaped_viewport_blocks(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    YuStorageShapedViewportSnapshot *snapshot,
    YuStorageShapedViewportBlock *blocks, size_t capacity, size_t *written);
int32_t yu_storage_session_macos_visual_decorations(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, float size, float max_width,
    float scroll_y, float viewport_height,
    YuStorageMacosVisualDecorationSnapshot *snapshot,
    YuStorageMacosVisualDecorationCaret *caret,
    YuStorageMacosVisualDecorationRect *rects, size_t capacity,
    size_t *written);
int32_t yu_storage_session_macos_shaped_caret_scroll_request(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height, float margin,
    YuStorageCaretScrollRequest *output);
int32_t yu_storage_session_macos_visual_scene(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    YuStorageVisualSceneSnapshot *snapshot,
    YuStorageVisualScenePrimitive *primitives, size_t capacity, size_t *written);
int32_t yu_storage_session_macos_visual_images(
    YuStorageSession *session, uint64_t expected_revision,
    YuStorageVisualImage *images, size_t capacity, size_t *written);
int32_t yu_storage_session_macos_visual_render_plan(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    YuStorageVisualRenderPlanSnapshot *snapshot,
    YuStorageVisualRenderCommand *commands, size_t command_capacity,
    YuStorageVisualRenderPage *pages, size_t page_capacity,
    YuStorageVisualRenderDamage *damage, size_t damage_capacity,
    size_t *written_commands, size_t *written_pages, size_t *written_damage);
int32_t yu_storage_session_macos_render_host_frame(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    uint64_t surface_generation, YuStorageMacosRenderHostSnapshot *snapshot);
int32_t yu_storage_session_macos_render_host_surface_submit(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    double surface_width, double surface_height, double scale, void *view,
    YuStorageMacosRenderHostSurfaceSnapshot *snapshot);
int32_t yu_storage_session_macos_render_host_surface_detach(
    YuStorageSession *session);
int32_t yu_storage_session_macos_visual_scene_glyphs(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    uint64_t surface_generation, YuStorageVisualSceneGlyphSnapshot *snapshot,
    YuStorageVisualSceneGlyph *glyphs, size_t capacity, size_t *written);
int32_t yu_storage_session_copy_source_range(const YuStorageSession *session,
                                             uint64_t expected_revision,
                                             uint64_t start_utf16,
                                             uint64_t end_utf16,
                                             uint8_t *output, size_t capacity,
                                             size_t *written);
int32_t yu_storage_session_copy_selection(const YuStorageSession *session,
                                          uint64_t expected_revision,
                                          uint8_t *output, size_t capacity,
                                          size_t *written);
int32_t yu_storage_session_copy_selection_html(const YuStorageSession *session,
                                               uint64_t expected_revision,
                                               uint8_t *output, size_t capacity,
                                               size_t *written);
/* Converts allowlisted HTML to Markdown; policy rejection must fall back to
 * the caller's text/plain payload. This function does not access a session. */
int32_t yu_storage_import_html_fragment(const uint8_t *html, size_t html_length,
                                        uint8_t *output, size_t capacity,
                                        size_t *written);
int32_t yu_storage_session_accessibility_snapshot(
    const YuStorageSession *session, YuStorageAccessibilitySnapshot *output);
int32_t yu_storage_session_accessibility_semantic_node_count(
    const YuStorageSession *session, uint64_t expected_revision, size_t *output);
int32_t yu_storage_session_accessibility_semantic_nodes(
    const YuStorageSession *session, uint64_t expected_revision,
    YuStorageAccessibilityNode *output, size_t capacity, size_t *written);
int32_t yu_storage_session_accessibility_semantic_nodes_v2(
    const YuStorageSession *session, uint64_t expected_revision,
    YuStorageAccessibilityNodeV2 *output, size_t capacity, size_t *written);
int32_t yu_storage_session_accessibility_line_range(
    const YuStorageSession *session, uint64_t expected_revision, uint64_t line,
    YuStorageAccessibilityRange *output);
int32_t yu_storage_session_accessibility_line_for_position(
    const YuStorageSession *session, uint64_t expected_revision,
    uint64_t offset_utf16, uint64_t *output);

int32_t yu_storage_session_selection(const YuStorageSession *session,
                                     YuStorageSelection *output);
int32_t yu_storage_session_selection_endpoints(
    const YuStorageSession *session, YuStorageSelectionEndpoints *output);
int32_t yu_storage_session_set_selection(YuStorageSession *session,
                                         uint64_t expected_revision,
                                         uint64_t start_utf16,
                                         uint64_t end_utf16,
                                         uint8_t affinity);
int32_t yu_storage_session_set_selection_endpoints(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t anchor_utf16, uint64_t focus_utf16, uint8_t affinity);
int32_t yu_storage_session_execute_command(YuStorageSession *session,
                                            uint8_t command, uint64_t block,
                                            YuStorageCommandResult *output);
int32_t yu_storage_session_macos_move_vertical(
    YuStorageSession *session, uint64_t expected_revision, uint8_t command,
    float size, float max_width, YuStorageCommandResult *output);
int32_t yu_storage_session_command_available(const YuStorageSession *session,
                                             uint8_t command, uint64_t block,
                                             uint8_t *output);
int32_t yu_storage_session_route_key(YuStorageSession *session, uint8_t key_kind,
                                     uint32_t key, uint8_t modifiers,
                                     YuStorageCommandResult *output);
int32_t yu_storage_session_insert_text(YuStorageSession *session,
                                        uint64_t expected_revision,
                                        const uint8_t *text, size_t text_length,
                                        YuStorageCommandResult *output);
int32_t yu_storage_session_composition(
    const YuStorageSession *session, YuStorageCompositionState *output);
int32_t yu_storage_session_copy_composition(
    const YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, uint8_t *output, size_t capacity,
    size_t *written);
int32_t yu_storage_session_begin_composition(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t replacement_start_utf16,
    uint64_t replacement_end_utf16, const uint8_t *preedit,
    size_t preedit_length, uint64_t selection_start_utf16,
    uint64_t selection_end_utf16);
int32_t yu_storage_session_update_composition(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, const uint8_t *preedit, size_t preedit_length,
    uint64_t selection_start_utf16, uint64_t selection_end_utf16);
int32_t yu_storage_session_commit_composition(YuStorageSession *session,
                                              uint64_t expected_revision,
                                              uint64_t expected_generation,
                                              const uint8_t *committed_text,
                                              size_t committed_length);
int32_t yu_storage_session_cancel_composition(YuStorageSession *session,
                                              uint64_t expected_revision,
                                              uint64_t expected_generation);

int32_t yu_storage_session_state(const YuStorageSession *session,
                                 YuStorageState *output);
int32_t yu_storage_session_save(YuStorageSession *session,
                                uint64_t *revision_output,
                                size_t *bytes_written_output,
                                uint8_t *changed_output);
int32_t yu_storage_session_reload(YuStorageSession *session,
                                  uint64_t *revision_output);

int32_t yu_storage_session_request_close(YuStorageSession *session,
                                          YuStorageCloseRequest *output);
int32_t yu_storage_session_cancel_close(YuStorageSession *session);
int32_t yu_storage_session_save_close(YuStorageSession *session);
int32_t yu_storage_session_discard_close(YuStorageSession *session);
int32_t yu_storage_session_save_failed_external(YuStorageSession *session,
                                                uint8_t external_state);

#endif
