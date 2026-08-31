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
    YU_STORAGE_TABLE_RESIZE_NOT_ACTIVE = 22,
};

enum {
    YU_STORAGE_SCENE_PRIMITIVE_BACKGROUND = 0,
    YU_STORAGE_SCENE_PRIMITIVE_TEXT_BOUNDS = 1,
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

/* Clipboard payload format for yu_storage_session_copy_selection. */
enum {
    YU_STORAGE_CLIPBOARD_TEXT = 0,
    YU_STORAGE_CLIPBOARD_HTML = 1,
};

/* Which axis a table divider belongs to. */
enum {
    YU_STORAGE_TABLE_RESIZE_NONE = 0,
    YU_STORAGE_TABLE_RESIZE_COLUMN = 1,
    YU_STORAGE_TABLE_RESIZE_ROW = 2,
};

/* Actions for yu_storage_session_macos_table_resize_at_point. */
enum {
    YU_STORAGE_TABLE_RESIZE_PROBE = 0,
    YU_STORAGE_TABLE_RESIZE_BEGIN = 1,
};

/* Actions for yu_storage_session_table_resize_action. */
enum {
    YU_STORAGE_TABLE_RESIZE_UPDATE = 0,
    YU_STORAGE_TABLE_RESIZE_FINISH = 1,
    YU_STORAGE_TABLE_RESIZE_CANCEL = 2,
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
    YU_STORAGE_CLOSE_RESOLVE_CANCEL = 0,
    YU_STORAGE_CLOSE_RESOLVE_SAVE = 1,
    YU_STORAGE_CLOSE_RESOLVE_DISCARD = 2,
};

enum {
    YU_STORAGE_ACCESSIBILITY_PARENT_NONE = UINT32_MAX,
    /* 大纲里没有上一级标题的那几条（文档的根级标题）。 */
    YU_STORAGE_OUTLINE_PARENT_NONE = UINT32_MAX,
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


/* Revision-bound selection endpoints. The anchor/focus pair preserves the
 * direction of a native visual drag; the ordered range is derived by the
 * caller from min/max and does not need its own ABI entry. */
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






/* One source-backed task checkbox from the currently published macOS retained
 * frame. Bounds use document-space scene coordinates; the marker range is the
 * exact parser-owned [ ]/[x] source range. */
typedef struct YuStorageTaskCheckboxHit {
    uint64_t revision;
    uint64_t block_index;
    uint64_t marker_start_utf16;
    uint64_t marker_end_utf16;
    float x;
    float y;
    float width;
    float height;
} YuStorageTaskCheckboxHit;

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

/* Revision-bound, document-space metadata for one visible table column
 * divider. This is a read-only Accessibility/inspection descriptor; it does
 * not open a resize gesture or mutate source, selection, history or layout.
 * When a session-only column preview exists, geometry reflects that preview. */
typedef struct YuStorageTableResizeAccessibilityDivider {
    uint64_t revision;
    uint64_t block_index;
    uint8_t kind;
    uint64_t index;
    uint64_t column_count;
    float x;
    float y;
    float width;
    float height;
    uint64_t table_source_start_utf16;
    uint64_t table_source_end_utf16;
    /* Column-width step for one VoiceOver increment/decrement, derived from the
     * table's own row height. This is a policy, not platform information: the
     * platform used to query font metrics through a dedicated FFI just to
     * compute it (I3). */
    float adjust_step;
} YuStorageTableResizeAccessibilityDivider;

/* Revision-bound, source-neutral geometry produced by a native table resize
 * gesture. update returns a preview; finish returns the final candidate. */
typedef struct YuStorageTableResizeCommit {
    uint64_t revision;
    uint64_t block_index;
    uint8_t kind;
    uint64_t index;
    float initial_position;
    float final_position;
    float delta;
} YuStorageTableResizeCommit;



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

enum {
    YU_STORAGE_RENDER_COMMAND_FILL_RECT = 0,
    YU_STORAGE_RENDER_COMMAND_GLYPH = 1,
    YU_STORAGE_RENDER_COMMAND_IMAGE = 2,
    YU_STORAGE_RENDER_COMMAND_EMBEDDED_SVG = 3,
    YU_STORAGE_RENDER_COMMAND_TASK_CHECKBOX = 4,
    YU_STORAGE_RENDER_PAGE_NONE = UINT32_MAX,
    YU_STORAGE_IMAGE_DESTINATION_NONE = UINT64_MAX,
    YU_STORAGE_IMAGE_INLINE = 0,
    YU_STORAGE_IMAGE_REFERENCE = 1,
    YU_STORAGE_IMAGE_RESOURCE_UNKNOWN = 0,
    YU_STORAGE_IMAGE_RESOURCE_PENDING = 1,
    YU_STORAGE_IMAGE_RESOURCE_READY = 2,
    YU_STORAGE_IMAGE_RESOURCE_FAILED = 3,
    YU_STORAGE_EMBEDDED_RESOURCE_UNKNOWN = 0,
    YU_STORAGE_EMBEDDED_RESOURCE_PENDING = 1,
    YU_STORAGE_EMBEDDED_RESOURCE_READY = 2,
    YU_STORAGE_EMBEDDED_RESOURCE_FAILED = 3,
    YU_STORAGE_EMBEDDED_RESOURCE_UNSUPPORTED = 4,
    YU_STORAGE_EMBEDDED_MATH = 0,
    YU_STORAGE_EMBEDDED_MERMAID = 1,
};







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
    uint64_t selection_decoration_count;
    uint64_t caret_decoration_count;
    uint64_t search_decoration_count;
    /* How many glyphs in this frame carry a code-highlight color (a color
     * other than this frame's body text color). This is the real-window
     * criterion for syntax highlighting: it is counted from scene primitives,
     * not from the decoration path that produced it. */
    uint64_t highlighted_glyph_count;
    /* Non-zero when the visible range still has an image or embedded resource
     * that has not settled. The platform schedules one more submit so Rust can
     * drain its worker results; it does not classify resource states itself. */
    uint8_t resource_refresh_pending;
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
    uint64_t selection_decoration_count;
    uint64_t caret_decoration_count;
    uint64_t search_decoration_count;
    uint64_t highlighted_glyph_count;
    uint8_t resource_refresh_pending;
    /* Rendered document height for this frame. The scrollable extent must come
     * from here: the platform has no second layout to derive it from (I5). */
    float content_height;
} YuStorageMacosRenderHostSurfaceSnapshot;

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

/* 大纲里的一条标题。与 YuStorageAccessibilityNodeV2 是并列的两份派生视图，
 * 不是一份套着另一份：语义树是扁平的（每个块都挂在 Document 下），大纲的
 * 全部内容恰恰是标题之间的层级。
 *
 * 导航不另开入口：拿 label_start_utf16 调
 * yu_storage_session_set_selection_endpoints，再调
 * yu_storage_session_macos_shaped_caret_scroll_request。 */
typedef struct YuStorageOutlineItem {
    uint64_t revision;
    uint32_t index;
    uint32_t parent;
    uint8_t level;
    uint8_t reserved[7];
    uint64_t block;
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t label_start_utf16;
    uint64_t label_end_utf16;
} YuStorageOutlineItem;

/* 一段被装饰藏起来的 source，UTF-16。
 *
 * 面板上的一行文字要不带语法标记（`## **粗** 标题` 显示成 `粗 标题`）。
 * 「哪几段被藏了」的唯一实现在 Rust 的 DecorationSet 里（不变量 D1），而画字
 * 的是 AppKit。把视觉文本拷过来会破 C4「parser 不复制正文」与整套 range-backed
 * 设计，所以交出的是**区间**：平台拿自己的 canonical 镜像按它们减掉。
 *
 * 区间升序、不重叠、不相邻。 */
typedef struct YuStorageHiddenSpan {
    uint64_t start_utf16;
    uint64_t end_utf16;
} YuStorageHiddenSpan;

/* 一处搜索命中。
 *
 * block 与 block_*_utf16 让调用方能守住 yu_storage_session_block_hidden_spans
 * 的那条规则（请求要落在一个块里）：结果面板上那一行未必与块边界对齐，有了
 * 块的区间，平台可以自己把行裁到块里。匹配跨块时 block 是起点所在的那一块。 */
typedef struct YuStorageSearchMatch {
    uint64_t revision;
    uint64_t block;
    uint64_t start_utf16;
    uint64_t end_utf16;
    uint64_t block_start_utf16;
    uint64_t block_end_utf16;
} YuStorageSearchMatch;

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

int32_t yu_storage_session_copy_path(const YuStorageSession *session,
                                     uint8_t *output, size_t capacity,
                                     size_t *written);
int32_t yu_storage_session_copy_source(const YuStorageSession *session,
                                       uint8_t *output, size_t capacity,
                                       size_t *written);
int32_t yu_storage_session_projection_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t source_utf16, uint8_t affinity,
    YuStorageProjectionCaret *output);
int32_t yu_storage_session_projection_source_selection(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t visual_start_utf16, uint64_t visual_end_utf16,
    uint8_t affinity, YuStorageProjectionSourceSelection *output);
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
int32_t yu_storage_session_macos_composition_shaped_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t expected_generation, uint64_t source_utf16, uint8_t affinity,
    float size, float max_width, YuStorageCompositionShapedCaret *output);
/* Advances one table divider drag. update/finish/cancel were three separate
 * entry points with identical parameters and preconditions; they are three
 * phases of one pointer gesture, which I3 already allows as an input event.
 * `pointer_position` is read only for UPDATE. `output` must be writable for
 * every action: failure paths clear it first and never leave a half-written
 * value (I4). */
int32_t yu_storage_session_table_resize_action(
    YuStorageSession *session, uint64_t expected_revision, uint8_t action,
    float pointer_position, YuStorageTableResizeCommit *output);
/* Probes or begins a table divider drag from one document-space point. The
 * read-only probe (hover) and the gesture start differed only by
 * `pointer_position`; they are two uses of one hit test. PROBE never mutates
 * session state and never reads `pointer_position`. */
int32_t yu_storage_session_macos_table_resize_at_point(
    YuStorageSession *session, uint64_t expected_revision, uint8_t action,
    float size, float max_width, float point_x, float point_y,
    float tolerance, float pointer_position, YuStorageTableResizeHit *output);
/* Resolves a document-space point against the exact task checkbox geometry in
 * the current persistent macOS render-host publication. It is read-only and
 * rejects stale revisions, active composition and points outside a checkbox. */
int32_t yu_storage_session_macos_task_checkbox_hit_test(
    YuStorageSession *session, uint64_t expected_revision,
    float point_x, float point_y, YuStorageTaskCheckboxHit *output);
/* Resolves a source caret's shaped geometry without the caller naming a
 * block. The platform needs this for IME candidate-window placement: only the
 * Rust layout knows where the caret is on screen, because TextKit lays out
 * canonical source while the screen shows the projection. */
int32_t yu_storage_session_macos_source_caret(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t source_utf16, uint8_t affinity,
    float size, float max_width, YuStorageBlockCaret *output);
/* Returns read-only, document-space descriptors for visible table column
 * dividers. The first call may use null output/zero capacity to query count;
 * an existing session-only column preview is reflected, but no resize session
 * is opened and source remains unchanged. */
int32_t yu_storage_session_macos_table_resize_accessibility_dividers(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    YuStorageTableResizeAccessibilityDivider *dividers, size_t capacity,
    size_t *written);
int32_t yu_storage_session_macos_shaped_caret_scroll_request(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    YuStorageCaretScrollRequest *output);
int32_t yu_storage_session_macos_render_host_frame(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    uint64_t surface_generation, YuStorageMacosRenderHostSnapshot *snapshot);
/* Geometry the platform supplies for one frame submission. Only AppKit knows
 * these values (view bounds, clip view scroll offset, backing scale); every
 * other decision stays in Rust. */
typedef struct YuStorageFrameGeometry {
    float size;
    float max_width;
    float scroll_y;
    float viewport_height;
    double surface_width;
    double surface_height;
    double scale;
} YuStorageFrameGeometry;
/* Reports whether submitting the next frame with this geometry would be
 * equivalent to the frame already on screen. Revision, composition generation
 * and geometry must all match; marked-text updates do not advance the
 * Revision, so the generation participates in the comparison. */
int32_t yu_storage_session_macos_frame_is_current(
    YuStorageSession *session, const YuStorageFrameGeometry *geometry,
    uint8_t *out_current);
int32_t yu_storage_session_macos_render_host_surface_submit(
    YuStorageSession *session, uint64_t expected_revision, float size,
    float max_width, float scroll_y, float viewport_height,
    double surface_width, double surface_height, double scale, void *view,
    YuStorageMacosRenderHostSurfaceSnapshot *snapshot);
int32_t yu_storage_session_macos_render_host_surface_detach(
    YuStorageSession *session);
int32_t yu_storage_session_copy_source_range(const YuStorageSession *session,
                                             uint64_t expected_revision,
                                             uint64_t start_utf16,
                                             uint64_t end_utf16,
                                             uint8_t *output, size_t capacity,
                                             size_t *written);
/* Copies the current selection in the requested format. Plain text and the
 * HTML fragment were two entry points with identical parameters differing only
 * in output format; the clipboard is one selection in several
 * representations. */
int32_t yu_storage_session_copy_selection(const YuStorageSession *session,
                                          uint64_t expected_revision,
                                          uint8_t format,
                                          uint8_t *output, size_t capacity,
                                          size_t *written);
/* Converts allowlisted HTML to Markdown; policy rejection must fall back to
 * the caller's text/plain payload. This function does not access a session. */
int32_t yu_storage_import_html_fragment(const uint8_t *html, size_t html_length,
                                        uint8_t *output, size_t capacity,
                                        size_t *written);
int32_t yu_storage_session_accessibility_snapshot(
    const YuStorageSession *session, YuStorageAccessibilitySnapshot *output);
int32_t yu_storage_session_accessibility_semantic_nodes_v2(
    const YuStorageSession *session, uint64_t expected_revision,
    YuStorageAccessibilityNodeV2 *output, size_t capacity, size_t *written);
/* 拷出这一版的大纲。两遍协议：output 传 NULL、capacity 传 0 时只回报条数。 */
int32_t yu_storage_session_outline_items(
    const YuStorageSession *session, uint64_t expected_revision,
    YuStorageOutlineItem *output, size_t capacity, size_t *written);
/* 换一份搜索查询，立刻在当前源码上扫出全部匹配。text 传 NULL、text_length
 * 传 0 表示收掉搜索。不校验 Revision：查询与源码正交。 */
int32_t yu_storage_session_set_search_query(
    YuStorageSession *session, const uint8_t *text, size_t text_length);
/* 拷出当前查询的全部匹配，按文档顺序，互不重叠。两遍协议。
 * 「跳到下一个」不在这里：那是一次导航，走已有的
 * yu_storage_session_set_selection_endpoints。 */
int32_t yu_storage_session_search_matches(
    const YuStorageSession *session, uint64_t expected_revision,
    YuStorageSearchMatch *output, size_t capacity, size_t *written);
/* 一个块里被藏起来的 source 区间，裁到 [start_utf16, end_utf16) 之内。
 * 两遍协议。请求区间必须整个落在这个块里；跨块的一行要按块问几次。 */
int32_t yu_storage_session_block_hidden_spans(
    YuStorageSession *session, uint64_t expected_revision, uint64_t block,
    uint64_t start_utf16, uint64_t end_utf16,
    YuStorageHiddenSpan *output, size_t capacity, size_t *written);
int32_t yu_storage_session_accessibility_line_range(
    const YuStorageSession *session, uint64_t expected_revision, uint64_t line,
    YuStorageAccessibilityRange *output);
int32_t yu_storage_session_accessibility_line_for_position(
    const YuStorageSession *session, uint64_t expected_revision,
    uint64_t offset_utf16, uint64_t *output);

int32_t yu_storage_session_selection_endpoints(
    const YuStorageSession *session, YuStorageSelectionEndpoints *output);
int32_t yu_storage_session_set_selection_endpoints(
    YuStorageSession *session, uint64_t expected_revision,
    uint64_t anchor_utf16, uint64_t focus_utf16, uint8_t affinity);
/* 全部选区的端点，按文档顺序，外加主选区的下标。两遍协议：output 传 NULL
 * （capacity 为 0）时只把条数写进 written。单数入口
 * yu_storage_session_selection_endpoints 仍然留着，它给的是 primary。 */
int32_t yu_storage_session_selections(
    const YuStorageSession *session, uint64_t expected_revision,
    YuStorageSelectionEndpoints *output, size_t capacity, size_t *written,
    size_t *primary);
/* 换一组选区。归一化（排序、合并、定位 primary）在 Rust 侧一家做：平台送来的
 * 可以是逆序、重叠、同一个偏移两次。count 为 0 是错误——永远至少有一条选区。 */
int32_t yu_storage_session_set_selections(
    YuStorageSession *session, uint64_t expected_revision,
    const YuStorageSelectionEndpoints *ranges, size_t count, size_t primary);
int32_t yu_storage_session_execute_command(YuStorageSession *session,
                                            uint8_t command, uint64_t block,
                                            YuStorageCommandResult *output);
int32_t yu_storage_session_macos_move_vertical(
    YuStorageSession *session, uint64_t expected_revision, uint8_t command,
    float size, float max_width, YuStorageCommandResult *output);
int32_t yu_storage_session_command_available(const YuStorageSession *session,
                                             uint8_t command, uint64_t block,
                                             uint8_t *output);
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
/* Ends one close negotiation. cancel/save/discard were three separate entry
 * points with identical parameters, preconditions and status codes; they are
 * three exits from one negotiation, which I3 already allows as a file
 * operation. `request_close` stays separate: it is the query that starts the
 * negotiation and reports whether the user must be asked. */
int32_t yu_storage_session_close_resolve(YuStorageSession *session,
                                          uint8_t action);

#endif
