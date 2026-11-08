/* Linked host gate: real caller pthread + real external usbmux/Lockdown/TLS peer.
 * Behavioral assertions follow original pthread join and the peer-join receipt.
 * Requires POSIX pthreads; other platform counterparts remain NOT_RUN. */
#define _POSIX_C_SOURCE 200809L
#include "ios_rs.h"
#include <assert.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(IOS_DEVICE_INFO_ALL == 7, "finite selector mask");
_Static_assert(offsetof(IosDeviceSessionConfig, udid) == 0, "config ABI");
_Static_assert(offsetof(IosDeviceSessionConfig, udid_len) == sizeof(char *), "config ABI");
_Static_assert(offsetof(IosDeviceQueryOptions, total_ms) == 0, "options ABI");
_Static_assert(offsetof(IosDeviceQueryOptions, reply_wire_bytes) ==
               ((sizeof(uint64_t) + _Alignof(size_t) - 1) / _Alignof(size_t)) * _Alignof(size_t), "options ABI");

static void wipe(void *pointer, size_t count) {
    volatile unsigned char *bytes = pointer;
    while (count--) *bytes++ = 0;
}

static unsigned char *read_pair(size_t *length) {
    unsigned char header[4];
    if (fread(header, 1, 4, stdin) != 4) return NULL;
    *length = ((size_t)header[0] << 24) | ((size_t)header[1] << 16) |
              ((size_t)header[2] << 8) | header[3];
    if (*length == 0 || *length > 1024U * 1024U) return NULL;
    unsigned char *bytes = malloc(*length);
    if (!bytes) return NULL;
    if (fread(bytes, 1, *length, stdin) != *length) {
        wipe(bytes, *length);
        free(bytes);
        return NULL;
    }
    return bytes;
}

static int command(const char *expected) {
    char input[64];
    return fgets(input, sizeof input, stdin) && strcmp(input, expected) == 0;
}

typedef struct Worker {
    IosDeviceSession *session;
    IosOperationCancel *cancel;
    IosDeviceQueryOptions options;
    uint32_t fields;
    size_t count;
    int status[2];
    const char *category[2];
    char *json[2];
} Worker;

static void *query(void *opaque) {
    Worker *worker = opaque;
    for (size_t index = 0; index < worker->count; ++index) {
        worker->status[index] = ios_device_session_query_json(
            worker->session, worker->fields, &worker->options,
            worker->cancel, &worker->json[index]);
        /* Snapshot on the exact original C caller thread before later status calls. */
        worker->category[index] = ios_last_error_class();
    }
    return NULL;
}

static void signatures(void) {
    int (*create)(const IosDeviceSessionConfig *, IosDeviceSession **) = ios_device_session_create;
    int (*retain)(const IosDeviceSession *, IosDeviceSession **) = ios_device_session_retain;
    int (*close_session)(const IosDeviceSession *, uint64_t) = ios_device_session_close;
    int (*destroy)(IosDeviceSession **) = ios_device_session_destroy_checked;
    int (*defaults)(IosDeviceQueryOptions *) = ios_device_query_options_default;
    int (*query_json)(const IosDeviceSession *, uint32_t, const IosDeviceQueryOptions *, const IosOperationCancel *, char **) = ios_device_session_query_json;
    int (*token_create)(IosOperationCancel **) = ios_operation_cancel_create;
    int (*token_retain)(const IosOperationCancel *, IosOperationCancel **) = ios_operation_cancel_retain;
    int (*signal)(const IosOperationCancel *) = ios_operation_cancel;
    int (*token_destroy)(IosOperationCancel **) = ios_operation_cancel_destroy_checked;
    assert(create && retain && close_session && destroy && defaults && query_json &&
           token_create && token_retain && signal && token_destroy);
}

static void contracts(IosDeviceSessionConfig config) {
    IosDeviceSession *session = NULL;
    IosOperationCancel *cancel = NULL;
    IosDeviceQueryOptions options;
    char *json = NULL;
    assert(ios_device_session_create(NULL, &session) == IOS_ERR_NULL && session == NULL);
    assert(ios_device_session_retain(NULL, &session) == IOS_ERR_NULL && session == NULL);
    assert(ios_device_query_options_default(NULL) == IOS_ERR_NULL);
    assert(ios_device_session_close(NULL, 1) == IOS_ERR_NULL);
    assert(ios_device_session_destroy_checked(NULL) == IOS_ERR_NULL);
    assert(ios_operation_cancel_create(NULL) == IOS_ERR_NULL);
    assert(ios_operation_cancel_retain(NULL, &cancel) == IOS_ERR_NULL);
    assert(ios_operation_cancel(NULL) == IOS_ERR_NULL);
    assert(ios_operation_cancel_destroy_checked(NULL) == IOS_ERR_NULL);
    for (int index = 0; index < 2; ++index) {
        assert(ios_device_session_destroy_checked(&session) == IOS_SUCCESS && session == NULL);
        assert(ios_operation_cancel_destroy_checked(&cancel) == IOS_SUCCESS && cancel == NULL);
    }
    const uint32_t local = config.concurrent_operations;
    config.concurrent_operations = UINT32_MAX;
    assert(ios_device_session_create(&config, &session) == IOS_ERR_CORE && session == NULL);
    assert(strcmp(ios_last_error_class(), "invalid_input") == 0);
    config.concurrent_operations = local;
    assert(ios_device_session_create(&config, &session) == IOS_SUCCESS);
    assert(ios_operation_cancel_create(&cancel) == IOS_SUCCESS);
    assert(ios_device_query_options_default(&options) == IOS_SUCCESS);
    for (uint32_t fields = 0; fields <= 8; fields += 8) {
        assert(ios_device_session_query_json(session, fields, &options, cancel, &json) == IOS_ERR_CORE && json == NULL);
        assert(strcmp(ios_last_error_class(), "invalid_input") == 0);
    }
    options.total_ms = UINT64_MAX;
    assert(ios_device_session_query_json(session, 7, &options, cancel, &json) == IOS_ERR_CORE && json == NULL);
    assert(strcmp(ios_last_error_class(), "invalid_input") == 0);
    assert(ios_device_session_close(session, UINT64_MAX) == IOS_ERR_CORE);
    assert(ios_device_query_options_default(&options) == IOS_SUCCESS);
    options.total_ms = 0;
    assert(ios_device_session_query_json(session, 7, &options, cancel, &json) == IOS_ERR_CORE && json == NULL);
    assert(strcmp(ios_last_error_class(), "timeout") == 0);
    assert(ios_device_query_options_default(&options) == IOS_SUCCESS);
    assert(ios_operation_cancel(cancel) == IOS_SUCCESS);
    assert(ios_device_session_query_json(session, 7, &options, cancel, &json) == IOS_ERR_CORE && json == NULL);
    assert(strcmp(ios_last_error_class(), "cancelled") == 0);
    assert(ios_device_session_close(session, 0) == IOS_SUCCESS);
    assert(ios_device_session_query_json(session, 7, &options, cancel, &json) == IOS_ERR_CORE && json == NULL);
    assert(strcmp(ios_last_error_class(), "precondition") == 0);
    assert(ios_operation_cancel_destroy_checked(&cancel) == IOS_SUCCESS);
    assert(ios_device_session_destroy_checked(&session) == IOS_SUCCESS);
    puts("C_CONTRACTS_JOINED");
    fflush(stdout);
    if (!command("peer_joined\n")) exit(2);
    puts("{\"status\":\"passed\",\"scenario\":\"contracts-no-source\"}");
}

int main(int argc, char **argv) {
    if (argc != 6) return 2;
    signatures();
    size_t pair_length = 0;
    unsigned char *pair = read_pair(&pair_length);
    if (!pair) return 2;
    const char *udid = "owned-ffi-session-device";
    IosDeviceSessionConfig config = {udid, strlen(udid), argv[1], strlen(argv[1]), pair, pair_length, 1};
    const char *mode = argv[2];
    const int expected = atoi(argv[3]);
    const char *expected_category = argv[4];
    const uint32_t fields = (uint32_t)strtoul(argv[5], NULL, 10);
    if (strcmp(mode, "contracts") == 0) {
        contracts(config);
        wipe(pair, pair_length);
        free(pair);
        return 0;
    }
    IosDeviceSession *session = NULL;
    IosDeviceSession *discard = NULL;
    IosOperationCancel *cancel = NULL;
    assert(ios_device_session_create(&config, &session) == IOS_SUCCESS);
    wipe(pair, pair_length);
    free(pair);
    /* Destroying one independent retained Box must leave the original open. */
    assert(ios_device_session_retain(session, &discard) == IOS_SUCCESS);
    assert(ios_device_session_destroy_checked(&discard) == IOS_SUCCESS && discard == NULL);
    assert(ios_operation_cancel_create(&cancel) == IOS_SUCCESS);
    Worker worker = {0};
    assert(ios_device_session_retain(session, &worker.session) == IOS_SUCCESS);
    assert(ios_operation_cancel_retain(cancel, &worker.cancel) == IOS_SUCCESS);
    assert(ios_device_query_options_default(&worker.options) == IOS_SUCCESS);
    worker.fields = fields;
    worker.count = strcmp(mode, "fresh") == 0 ? 2 : 1;
    if (strcmp(mode, "timeout") == 0) worker.options.total_ms = 1000;
    if (strcmp(mode, "wire-limit") == 0) worker.options.reply_wire_bytes = 16;
    if (strcmp(mode, "utf8-limit") == 0) worker.options.output_utf8_bytes = 3;
    if (strcmp(mode, "json-limit") == 0) worker.options.json_bytes = 1;
    pthread_t original;
    if (pthread_create(&original, NULL, query, &worker) != 0) return 2;
    int control_status = IOS_SUCCESS;
    const char *control_category = "none";
    int admission_status = IOS_SUCCESS;
    const char *admission_category = "none";
    int admission_output_null = 1;
    if (strcmp(mode, "cancel") == 0 || strcmp(mode, "close") == 0 || strcmp(mode, "admission") == 0) {
        if (!command("act\n")) return 2;
        if (strcmp(mode, "admission") == 0) {
            char *rejected = NULL;
            admission_status = ios_device_session_query_json(session, fields, &worker.options, cancel, &rejected);
            admission_category = ios_last_error_class();
            admission_output_null = rejected == NULL;
            /* Save only; assert after the original thread and peer were joined. */
            if (rejected != NULL) { ios_free_string(rejected); }
        }
        control_status = strcmp(mode, "close") == 0 ?
            ios_device_session_close(session, 2000) : ios_operation_cancel(cancel);
        control_category = ios_last_error_class();
    }
    const int original_join = pthread_join(original, NULL);
    puts("C_QUERY_THREAD_JOINED");
    fflush(stdout);
    if (!command("peer_joined\n")) return 2;
    /* The external runner issues that command only after strict original native
     * TCP EOF and joining its original peer worker(s), with evidence retained. */
    assert(original_join == 0);
    for (size_t index = 0; index < worker.count; ++index) {
        assert(worker.status[index] == expected);
        assert(strcmp(worker.category[index], expected_category) == 0);
        assert((worker.status[index] == IOS_SUCCESS) == (worker.json[index] != NULL));
    }
    assert(control_status == IOS_SUCCESS && strcmp(control_category, "none") == 0);
    if (strcmp(mode, "admission") == 0) {
        assert(admission_status == IOS_ERR_CORE && strcmp(admission_category, "conflict") == 0 && admission_output_null);
    }
    assert(ios_device_session_close(session, 2000) == IOS_SUCCESS);
    assert(ios_device_session_close(worker.session, 0) == IOS_SUCCESS);
    char *closed_output = NULL;
    assert(ios_device_session_query_json(session, IOS_DEVICE_INFO_ALL, &worker.options, cancel, &closed_output) == IOS_ERR_CORE);
    assert(closed_output == NULL && strcmp(ios_last_error_class(), "precondition") == 0);
    printf("{\"status\":\"passed\",\"query_status\":%d,\"query_class\":\"%s\",\"results\":[", expected, expected_category);
    for (size_t index = 0; index < worker.count; ++index) {
        printf("%s%s", index ? "," : "", worker.json[index] ? worker.json[index] : "null");
    }
    puts("],\"original_c_thread_joined\":true,\"peer_join_receipt_before_assertions\":true}");
    for (size_t index = 0; index < worker.count; ++index) {
        assert(ios_free_string_checked(&worker.json[index]) == IOS_SUCCESS);
        assert(ios_free_string_checked(&worker.json[index]) == IOS_SUCCESS);
    }
    assert(ios_device_session_destroy_checked(&worker.session) == IOS_SUCCESS);
    assert(ios_device_session_destroy_checked(&session) == IOS_SUCCESS);
    assert(ios_device_session_destroy_checked(&session) == IOS_SUCCESS);
    assert(ios_operation_cancel_destroy_checked(&worker.cancel) == IOS_SUCCESS);
    assert(ios_operation_cancel_destroy_checked(&cancel) == IOS_SUCCESS);
    assert(ios_operation_cancel_destroy_checked(&cancel) == IOS_SUCCESS);
    return 0;
}
