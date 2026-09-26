// Read-only kernel executable identity for the two exact isolated test binaries.
// Never use argv/ps command spelling to identify processes for sampling/cleanup.
#include <errno.h>
#include <libproc.h>
#include <limits.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/param.h>
#include <sys/proc.h>
#include <sys/proc_info.h>
#include <unistd.h>

#define MAX_PIDS 65536

struct identity {
    int kind;
    struct proc_bsdinfo info;
};

static int same_process(const struct proc_bsdinfo *a, const struct proc_bsdinfo *b) {
    return a->pbi_pid == b->pbi_pid && a->pbi_uid == b->pbi_uid
        && a->pbi_start_tvsec == b->pbi_start_tvsec
        && a->pbi_start_tvusec == b->pbi_start_tvusec;
}

static int gone(pid_t pid) {
    return kill(pid, 0) == -1 && errno == ESRCH;
}

int main(int argc, char **argv) {
    if (argc != 4) return 2;
    char *end = NULL;
    errno = 0;
    long owner = strtol(argv[3], &end, 10);
    if (errno || end == argv[3] || *end || owner < 0 || owner > INT_MAX) return 2;
    char expected[2][PATH_MAX];
    for (int i = 0; i < 2; i++) {
        if (argv[i + 1][0] != '/' || realpath(argv[i + 1], expected[i]) == NULL) {
            fprintf(stderr, "Cannot resolve an absolute isolated executable: %d\n", errno);
            return 2;
        }
    }
    if (strcmp(expected[0], expected[1]) == 0) return 2;
    pid_t *pids = calloc(MAX_PIDS, sizeof(*pids));
    struct identity *found = calloc(MAX_PIDS, sizeof(*found));
    if (!pids || !found) {
        free(pids); free(found); return 1;
    }
    // Only this user's processes can belong to the non-privileged test bundle.
    // A full buffer fails closed, rather than silently omitting later processes.
    int capacity = MAX_PIDS * (int)sizeof(*pids);
    int bytes = proc_listpids(PROC_UID_ONLY, (uint32_t)geteuid(), pids, capacity);
    int result = 1;
    size_t count = 0;
    unsigned unreadable_unrelated = 0;
    if (bytes <= 0 || bytes >= capacity || bytes % (int)sizeof(*pids) != 0) {
        fprintf(stderr, "Incomplete process inventory: bytes=%d errno=%d\n", bytes, errno);
        goto finish;
    }
    for (int i = 0; i < bytes / (int)sizeof(*pids); i++) {
        pid_t pid = pids[i];
        if (pid <= 0) continue;
        struct proc_bsdinfo before = {0}, after = {0};
        int n = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &before, sizeof(before));
        if (n != sizeof(before)) {
            if (gone(pid)) continue;
            if (owner && getpgid(pid) == owner) {
                fprintf(stderr, "Cannot identify live isolated group member %d\n", pid);
                goto finish;
            }
            unreadable_unrelated++;
            continue;
        }
        if (before.pbi_status == SZOMB) continue;
        char path[PROC_PIDPATHINFO_MAXSIZE] = {0};
        if (proc_pidpath(pid, path, sizeof(path)) <= 0) {
            // Recheck exit/zombie races. Other failures are not a zero count.
            n = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &after, sizeof(after));
            if (gone(pid) || (n == sizeof(after) && after.pbi_status == SZOMB)) continue;
            if (owner && (before.pbi_pgid == owner || before.pbi_ppid == owner)) {
                fprintf(stderr, "Cannot read kernel path of live isolated process %d\n", pid);
                goto finish;
            }
            // Some unrelated system-managed user processes deny this query.
            // Report that scope explicitly; never treat an unreadable member
            // of our own isolated group as an absent application/helper.
            unreadable_unrelated++;
            continue;
        }
        char canonical[PATH_MAX];
        if (realpath(path, canonical) == NULL) {
            // The exact bundle paths above exist throughout the check. An
            // unrelated unlinked executable cannot match either of them.
            if (strcmp(path, expected[0]) != 0 && strcmp(path, expected[1]) != 0) continue;
            fprintf(stderr, "Isolated executable disappeared for %d\n", pid);
            goto finish;
        }
        int kind = strcmp(canonical, expected[0]) == 0 ? 0
            : strcmp(canonical, expected[1]) == 0 ? 1 : -1;
        if (kind < 0) continue;
        n = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &after, sizeof(after));
        if (n != sizeof(after)) {
            if (gone(pid)) continue;
            fprintf(stderr, "Cannot recheck isolated process %d\n", pid);
            goto finish;
        }
        if (!same_process(&before, &after) || after.pbi_status == SZOMB) continue;
        // Fence an exec that happened while reading the BSD information.
        if (proc_pidpath(pid, path, sizeof(path)) <= 0) {
            n = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &after, sizeof(after));
            if (gone(pid) || (n == sizeof(after) && after.pbi_status == SZOMB)) continue;
            fprintf(stderr, "Cannot recheck isolated executable %d\n", pid);
            goto finish;
        }
        if (realpath(path, canonical) == NULL || strcmp(canonical, expected[kind]) != 0) continue;
        found[count].kind = kind;
        found[count++].info = after;
    }
    printf("{\"schema_version\":1,\"unreadable_unrelated\":%u,\"processes\":[", unreadable_unrelated);
    for (size_t i = 0; i < count; i++) {
        const struct proc_bsdinfo *info = &found[i].info;
        printf("%s{\"kind\":\"%s\",\"pid\":%u,\"ppid\":%u,\"pgid\":%u,"
               "\"start_seconds\":%llu,\"start_microseconds\":%llu}",
               i ? "," : "", found[i].kind == 0 ? "app" : "helpers",
               info->pbi_pid, info->pbi_ppid, info->pbi_pgid,
               (unsigned long long)info->pbi_start_tvsec,
               (unsigned long long)info->pbi_start_tvusec);
    }
    printf("]}\n");
    result = 0;
finish:
    free(pids); free(found);
    return result;
}
