/* Explicit existing-trust information. Caller supplies already-loaded pair
 * bytes on stdin; the library performs no credential file/default lookup. */
#include "ios_rs.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void wipe(void *pointer, size_t count) {
    volatile unsigned char *bytes = pointer;
    while (count--) *bytes++ = 0;
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fputs("usage: device_info EXPLICIT_UDID tcp://NUMERIC_ADDRESS:PORT < already-loaded-pair.plist\n", stderr);
        return 2;
    }
    const size_t cap = 1024U * 1024U;
    unsigned char *pair = malloc(cap + 1);
    if (!pair) return 1;
    const size_t length = fread(pair, 1, cap + 1, stdin);
    if (ferror(stdin) || length == 0 || length > cap) {
        wipe(pair, length);
        free(pair);
        fputs("caller pairing input must be nonempty and at most 1MiB\n", stderr);
        return 1;
    }
    IosDeviceSessionConfig config = {
        argv[1], strlen(argv[1]), argv[2], strlen(argv[2]), pair, length, 1
    };
    IosDeviceSession *session = NULL;
    IosOperationCancel *cancel = NULL;
    char *json = NULL;
    int status = ios_device_session_create(&config, &session);
    const char *category = ios_last_error_class();
    wipe(pair, length);
    free(pair); /* Create has decoded/copied everything it needs. */
    if (status == IOS_SUCCESS) {
        status = ios_operation_cancel_create(&cancel);
        category = ios_last_error_class();
    }
    IosDeviceQueryOptions options;
    if (status == IOS_SUCCESS) {
        status = ios_device_query_options_default(&options);
        category = ios_last_error_class();
    }
    if (status == IOS_SUCCESS) {
        status = ios_device_session_query_json(session, IOS_DEVICE_INFO_ALL,
                                               &options, cancel, &json);
        category = ios_last_error_class();
    }
    if (status == IOS_SUCCESS) puts(json);
    else fprintf(stderr, "finite device query failed (%s)\n", category);
    int release = ios_free_string_checked(&json);
    int close_status = session ? ios_device_session_close(session, 5000) : IOS_SUCCESS;
    int token_release = ios_operation_cancel_destroy_checked(&cancel);
    int session_release = ios_device_session_destroy_checked(&session);
    return status == IOS_SUCCESS && release == IOS_SUCCESS &&
           close_status == IOS_SUCCESS && token_release == IOS_SUCCESS &&
           session_release == IOS_SUCCESS ? 0 : 1;
}
