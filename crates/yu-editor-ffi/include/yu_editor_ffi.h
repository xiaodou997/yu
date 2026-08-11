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
    YU_EDITOR_COMMAND_INSERT_NEWLINE = 5,
    YU_EDITOR_COMMAND_INDENT_LIST = 6,
    YU_EDITOR_COMMAND_OUTDENT_LIST = 7,
    YU_EDITOR_COMMAND_UNDO = 8,
    YU_EDITOR_COMMAND_REDO = 9,
    YU_EDITOR_COMMAND_TOGGLE_TASK = 10,
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
