/* Linux/glibc diagnostic overlay only; never linked into the product.
 * One sampler thread/FILE perturbs the workload. Timing is not benchmark data.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <malloc.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static FILE *output;
static pthread_t sampler;
static atomic_int stopped;
static struct timespec started;

static long status_field(const char *status, const char *name) {
    size_t length = strlen(name);
    for (const char *p = status; *p; p++) {
        long value;
        if ((p == status || p[-1] == '\n') && strncmp(p, name, length) == 0 &&
            sscanf(p + length, "%ld", &value) == 1) return value;
    }
    return -1;
}

static void sample(const char *phase, int trim_result) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    double seconds = (double)(now.tv_sec - started.tv_sec) +
                     (double)(now.tv_nsec - started.tv_nsec) / 1e9;
    long rss = -1, anon = -1, file = -1, shmem = -1;
    char status[16384];
    int fd = open("/proc/self/status", O_RDONLY | O_CLOEXEC);
    if (fd >= 0) {
        ssize_t count = read(fd, status, sizeof(status) - 1);
        close(fd);
        if (count > 0) {
            status[count] = '\0';
            rss = status_field(status, "VmRSS:");
            anon = status_field(status, "RssAnon:");
            file = status_field(status, "RssFile:");
            shmem = status_field(status, "RssShmem:");
        }
    }
    fprintf(output, "<sample phase=\"%s\" seconds=\"%.6f\" rss_kib=\"%ld\" anon_kib=\"%ld\" file_kib=\"%ld\" shmem_kib=\"%ld\" trim_result=\"%d\">\n",
            phase, seconds, rss, anon, file, shmem, trim_result);
    if (malloc_info(0, output) != 0) fputs("<error/>\n", output);
    fputs("</sample>\n", output);
}

static void *observe(void *unused) {
    (void)unused;
    const struct timespec pause = {.tv_sec = 0, .tv_nsec = 500000000};
    /* Bound output even if the caller fails to impose its process deadline. */
    for (int i = 0; i < 4096 && !atomic_load(&stopped); i++) {
        sample("periodic", -1);
        nanosleep(&pause, NULL);
    }
    return NULL;
}

__attribute__((constructor)) static void start_probe(void) {
    const char *path = getenv("IOS_MANAGER_ALLOCATOR_OUTPUT");
    if (!path || !*path) return;
    const char *expected = getenv("IOS_MANAGER_ALLOCATOR_EXECUTABLE");
    char executable[4096];
    ssize_t count = readlink("/proc/self/exe", executable, sizeof(executable) - 1);
    if (!expected || count < 0) return;
    executable[count] = '\0';
    if (strcmp(expected, executable) != 0) return;
    int fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (fd < 0) _exit(90);
    output = fdopen(fd, "w");
    if (!output) _exit(91);
    setvbuf(output, NULL, _IONBF, 0);
    clock_gettime(CLOCK_MONOTONIC, &started);
    fputs("<allocator_probe>\n", output);
    sample("startup", -1);
    if (pthread_create(&sampler, NULL, observe, NULL) != 0) _exit(92);
}

__attribute__((destructor)) static void finish_probe(void) {
    if (!output) return;
    atomic_store(&stopped, 1);
    pthread_join(sampler, NULL);
    sample("before_trim", -1);
    int released = malloc_trim(0);
    sample("after_trim", released);
    fputs("</allocator_probe>\n", output);
    fclose(output);
}
