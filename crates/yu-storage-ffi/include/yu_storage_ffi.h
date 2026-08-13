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
