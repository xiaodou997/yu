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

/* Revision-bound metrics-layout hit-test result. x/y are projection-local
 * layout coordinates, not screen coordinates. */
typedef struct YuStorageProjectionHit {
    uint64_t revision;
    uint64_t source_utf16;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
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
int32_t yu_storage_session_projection_hit_test(
    YuStorageSession *session, uint64_t expected_revision,
    float point_x, float point_y, float max_width, float line_height,
    float default_advance, YuStorageProjectionHit *output);
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
int32_t yu_storage_session_set_selection(YuStorageSession *session,
                                         uint64_t expected_revision,
                                         uint64_t start_utf16,
                                         uint64_t end_utf16,
                                         uint8_t affinity);
int32_t yu_storage_session_execute_command(YuStorageSession *session,
                                            uint8_t command, uint64_t block,
                                            YuStorageCommandResult *output);
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
