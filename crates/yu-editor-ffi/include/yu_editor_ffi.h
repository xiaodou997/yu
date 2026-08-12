#ifndef YU_EDITOR_FFI_H
#define YU_EDITOR_FFI_H

#include <stddef.h>
#include <stdint.h>

typedef struct YuCompositionSession YuCompositionSession;

enum {
    YU_FFI_OK = 0,
    YU_FFI_NULL_POINTER = 1,
    YU_FFI_INVALID_UTF8 = 2,
    YU_FFI_INVALID_RANGE = 3,
    YU_FFI_INVALID_SELECTION = 4,
    YU_FFI_NO_OVERLAY = 5,
    YU_FFI_BUFFER_TOO_SMALL = 6,
    YU_FFI_EDIT_FAILED = 7,
    YU_FFI_STALE_REVISION = 8,
    YU_FFI_KEY_UNHANDLED = 9,
    YU_FFI_INVALID_COMMAND = 10,
    YU_FFI_INVALID_KEY = 11,
    YU_FFI_INVALID_VIEWPORT_CONFIG = 12,
    YU_FFI_CORE_TEXT_UNAVAILABLE = 13,
    YU_FFI_LAYOUT_FAILED = 14,
};

enum {
    YU_COMMAND_UNAVAILABLE = 0,
    YU_COMMAND_AVAILABLE = 1,
};

enum {
    YU_CARET_AFFINITY_UPSTREAM = 0,
    YU_CARET_AFFINITY_DOWNSTREAM = 1,
};

enum {
    YU_KEY_CHARACTER = 0,
    YU_KEY_ENTER = 1,
    YU_KEY_TAB = 2,
    YU_KEY_BACKSPACE = 3,
    YU_KEY_DELETE = 4,
    YU_KEY_LEFT = 5,
    YU_KEY_RIGHT = 6,
    YU_KEY_UP = 7,
    YU_KEY_DOWN = 8,
    YU_KEY_ESCAPE = 9,
};

enum {
    YU_KEY_MODIFIER_COMMAND = 1 << 0,
    YU_KEY_MODIFIER_SHIFT = 1 << 1,
    YU_KEY_MODIFIER_CONTROL = 1 << 2,
    YU_KEY_MODIFIER_OPTION = 1 << 3,
};

enum {
    YU_SOURCE_SYNC_NONE = 0,
    YU_SOURCE_SYNC_RANGE = 1,
    YU_SOURCE_SYNC_FULL = 2,
};

enum {
    YU_EDITOR_COMMAND_DELETE_BACKWARD = 1,
    YU_EDITOR_COMMAND_DELETE_FORWARD = 2,
    YU_EDITOR_COMMAND_MOVE_LEFT = 3,
    YU_EDITOR_COMMAND_MOVE_RIGHT = 4,
    YU_EDITOR_COMMAND_MOVE_WORD_LEFT = 11,
    YU_EDITOR_COMMAND_MOVE_WORD_RIGHT = 12,
    YU_EDITOR_COMMAND_INSERT_NEWLINE = 5,
    YU_EDITOR_COMMAND_INDENT_LIST = 6,
    YU_EDITOR_COMMAND_OUTDENT_LIST = 7,
    YU_EDITOR_COMMAND_UNDO = 8,
    YU_EDITOR_COMMAND_REDO = 9,
    YU_EDITOR_COMMAND_TOGGLE_TASK = 10,
    YU_EDITOR_COMMAND_MOVE_UP = 13,
    YU_EDITOR_COMMAND_MOVE_DOWN = 14,
    YU_EDITOR_COMMAND_MOVE_UP_EXTEND = 15,
    YU_EDITOR_COMMAND_MOVE_DOWN_EXTEND = 16,
};

typedef struct YuEditorCommandResult {
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
} YuEditorCommandResult;

typedef struct YuEditorCaretScrollRequest {
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
} YuEditorCaretScrollRequest;

typedef struct YuCoreTextViewportMetrics {
    float line_height;
    float default_advance;
} YuCoreTextViewportMetrics;

typedef struct YuCoreTextShapedLine {
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    float width;
} YuCoreTextShapedLine;

typedef struct YuCoreTextProjectedLine {
    uint64_t source_start_utf16;
    uint64_t source_end_utf16;
    uint64_t visual_start_utf16;
    uint64_t visual_end_utf16;
    float width;
} YuCoreTextProjectedLine;

typedef struct YuProjectionCaret {
    uint64_t revision;
    uint64_t source_utf16;
    uint64_t visual_utf16;
    uint64_t round_trip_source_utf16;
    uint8_t affinity;
} YuProjectionCaret;

int32_t yu_composition_session_new(const uint8_t *source, size_t source_length,
                                   YuCompositionSession **output);
void yu_composition_session_destroy(YuCompositionSession *session);
int32_t yu_composition_session_reset_source(YuCompositionSession *session,
                                             const uint8_t *source,
                                             size_t source_length);
int32_t yu_composition_session_execute_command(YuCompositionSession *session,
                                               uint8_t command,
                                               uint64_t block,
                                               YuEditorCommandResult *output);
int32_t yu_composition_session_command_available(YuCompositionSession *session,
                                                 uint8_t command,
                                                 uint64_t block,
                                                 uint8_t *output);
int32_t yu_composition_session_route_key(YuCompositionSession *session,
                                         uint8_t key_kind,
                                         uint32_t key,
                                         uint8_t modifiers,
                                         YuEditorCommandResult *output);
int32_t yu_composition_session_begin(YuCompositionSession *session,
                                     uint64_t replacement_start_utf16,
                                     uint64_t replacement_end_utf16,
                                     const uint8_t *preedit,
                                     size_t preedit_length,
                                     uint64_t selection_start_utf16,
                                     uint64_t selection_end_utf16);
int32_t yu_composition_session_update(YuCompositionSession *session,
                                      const uint8_t *preedit,
                                      size_t preedit_length,
                                      uint64_t selection_start_utf16,
                                      uint64_t selection_end_utf16);
int32_t yu_composition_session_commit(YuCompositionSession *session,
                                      const uint8_t *committed_text,
                                      size_t committed_length);
int32_t yu_composition_session_cancel(YuCompositionSession *session);
int32_t yu_composition_session_revision(const YuCompositionSession *session,
                                        uint64_t *output);
int32_t yu_composition_session_selection(const YuCompositionSession *session,
                                         uint64_t *revision_output,
                                         uint64_t *start_output,
                                         uint64_t *end_output,
                                         uint8_t *affinity_output);
int32_t yu_composition_session_set_selection(YuCompositionSession *session,
                                             uint64_t expected_revision,
                                             uint64_t start_utf16,
                                             uint64_t end_utf16,
                                             uint8_t affinity);
int32_t yu_composition_session_projection_caret(YuCompositionSession *session,
                                                uint64_t expected_revision,
                                                uint64_t source_utf16,
                                                uint8_t affinity,
                                                YuProjectionCaret *output);
int32_t yu_composition_session_set_viewport_config(YuCompositionSession *session,
                                                   uint64_t expected_revision,
                                                   float max_width,
                                                   float line_height,
                                                   float default_advance,
                                                   float estimated_block_height,
                                                   float overscan);
int32_t yu_macos_core_text_viewport_metrics(const uint8_t *family,
                                            size_t family_length,
                                            float size,
                                            const uint8_t *sample,
                                            size_t sample_length,
                                            YuCoreTextViewportMetrics *output);
int32_t yu_macos_core_text_system_ui_viewport_metrics(
    float size, const uint8_t *sample, size_t sample_length,
    YuCoreTextViewportMetrics *output);
int32_t yu_macos_core_text_shaped_lines(
    float size, float max_width, const uint8_t *source, size_t source_length,
    YuCoreTextShapedLine *output, size_t capacity, size_t *written);
int32_t yu_macos_core_text_projected_layout(
    float size, float max_width, const uint8_t *source, size_t source_length,
    YuCoreTextProjectedLine *lines, size_t line_capacity, size_t *line_written,
    uint8_t *visual_output, size_t visual_capacity, size_t *visual_written);
int32_t yu_composition_session_caret_scroll_request(YuCompositionSession *session,
                                                    uint64_t expected_revision,
                                                    float scroll_y,
                                                    float viewport_height,
                                                    float margin,
                                                    YuEditorCaretScrollRequest *output);
int32_t yu_composition_session_source_length(const YuCompositionSession *session,
                                             size_t *output);
int32_t yu_composition_session_copy_source(const YuCompositionSession *session,
                                           uint8_t *output, size_t capacity);
int32_t yu_composition_session_source_range_length(const YuCompositionSession *session,
                                                    uint64_t expected_revision,
                                                    uint64_t start_utf16,
                                                    uint64_t end_utf16,
                                                    size_t *output);
int32_t yu_composition_session_copy_source_range(const YuCompositionSession *session,
                                                 uint64_t expected_revision,
                                                 uint64_t start_utf16,
                                                 uint64_t end_utf16,
                                                 uint8_t *output, size_t capacity,
                                                 size_t *written);
int32_t yu_composition_session_overlay_length(const YuCompositionSession *session,
                                              size_t *output);
int32_t yu_composition_session_copy_overlay(const YuCompositionSession *session,
                                            uint8_t *output, size_t capacity);
int32_t yu_composition_session_overlay_selection(const YuCompositionSession *session,
                                                 uint64_t *start_output,
                                                 uint64_t *end_output);

#endif
