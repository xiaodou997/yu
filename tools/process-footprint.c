// Read-only macOS physical-footprint sampling for a known test child process.
#include <errno.h>
#include <libproc.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    char *end = NULL;
    long value = strtol(argv[1], &end, 10);
    if (!end || *end || value <= 0 || value > 2147483647) return 2;
    struct rusage_info_v4 usage = {0};
    if (proc_pid_rusage((int)value, RUSAGE_INFO_V4, (rusage_info_t *)&usage) != 0) {
        fprintf(stderr, "proc_pid_rusage failed: %d\n", errno);
        return 1;
    }
    printf("{\"pid\":%ld,\"physical_footprint_bytes\":%llu,\"lifetime_peak_physical_footprint_bytes\":%llu}\n",
           value, (unsigned long long)usage.ri_phys_footprint,
           (unsigned long long)usage.ri_lifetime_max_phys_footprint);
    return 0;
}
