/* Read-only configured SDK example. Pair record bytes are supplied on stdin. */
#include "ios_rs.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s UDID numeric-endpoint [read-file] < pair.plist\n", argv[0]);
        return 2;
    }
    const size_t maximum_pair = 1024U * 1024U;
    uint8_t *pair = malloc(maximum_pair + 1);
    if (!pair) return 1;
    const size_t count = fread(pair, 1, maximum_pair + 1, stdin);
    if (ferror(stdin) || count == 0 || count > maximum_pair) {
        free(pair);
        fprintf(stderr, "pair record must contain 1..1MiB bytes\n");
        return 1;
    }
    IosDeviceSessionConfig config = {
        argv[1], strlen(argv[1]), argv[2], strlen(argv[2]), pair, count, 1
    };
    IosDeviceSession *session = NULL;
    IosOperationCancel *cancel = NULL;
    IosDeviceServiceOptions options;
    char *json = NULL;
    uint8_t *data = NULL;
    size_t length = 0;
    int status = ios_device_session_create(&config, &session);
    /* Caller retains and clears its input; the SDK zeroizes its parsed key. */
    volatile uint8_t *secret = pair;
    for (size_t index = 0; index < count; ++index) secret[index] = 0;
    free(pair);
    if (status == IOS_SUCCESS) status = ios_operation_cancel_create(&cancel);
    if (status == IOS_SUCCESS) status = ios_device_service_options_default(&options);
    options.total_ms = 5000;
    if (status == IOS_SUCCESS) {
        status = ios_device_session_service_json(session, 1, NULL, 0, 0, &options, cancel, &json);
        if (status == IOS_SUCCESS) printf("User applications: %s\n", json);
    }
    const char *operation_class = ios_last_error_class();
    if (status != IOS_SUCCESS) fprintf(stderr, "apps failed (%s)\n", operation_class);
    int cleanup = ios_free_string_checked(&json);
    if (status == IOS_SUCCESS && cleanup == IOS_SUCCESS) {
        status = ios_device_session_service_json(session, 2, "/", 1, 0, &options, cancel, &json);
        operation_class = ios_last_error_class();
        if (status == IOS_SUCCESS) printf("AFC directory: %s\n", json);
        else fprintf(stderr, "directory failed (%s)\n", operation_class);
    }
    if (ios_free_string_checked(&json) != IOS_SUCCESS) cleanup = 1;
    if (status == IOS_SUCCESS && cleanup == IOS_SUCCESS && argc > 3) {
        options.output_bytes = 65536;
        status = ios_device_session_afc_read_file(session, argv[3], strlen(argv[3]),
            65536, &options, cancel, &data, &length);
        operation_class = ios_last_error_class();
        if (status == IOS_SUCCESS) printf("Complete file bytes: %zu\n", length);
        else fprintf(stderr, "file read failed (%s)\n", operation_class);
    }
    if (ios_free_bytes_checked(&data, &length) != IOS_SUCCESS) cleanup = 1;
    if (session && ios_device_session_close(session, 5000) != IOS_SUCCESS) cleanup = 1;
    if (ios_device_session_destroy_checked(&session) != IOS_SUCCESS) cleanup = 1;
    if (ios_operation_cancel_destroy_checked(&cancel) != IOS_SUCCESS) cleanup = 1;
    return status == IOS_SUCCESS && cleanup == IOS_SUCCESS ? 0 : 1;
}
