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
};

int32_t yu_composition_session_new(const uint8_t *source, size_t source_length,
                                   YuCompositionSession **output);
void yu_composition_session_destroy(YuCompositionSession *session);
int32_t yu_composition_session_reset_source(YuCompositionSession *session,
                                             const uint8_t *source,
                                             size_t source_length);
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
int32_t yu_composition_session_source_length(const YuCompositionSession *session,
                                             size_t *output);
int32_t yu_composition_session_copy_source(const YuCompositionSession *session,
                                           uint8_t *output, size_t capacity);
int32_t yu_composition_session_overlay_length(const YuCompositionSession *session,
                                              size_t *output);
int32_t yu_composition_session_copy_overlay(const YuCompositionSession *session,
                                            uint8_t *output, size_t capacity);
int32_t yu_composition_session_overlay_selection(const YuCompositionSession *session,
                                                 uint64_t *start_output,
                                                 uint64_t *end_output);

#endif
