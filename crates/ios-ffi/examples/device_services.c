/* Device services example: apps, AFC files, and syslog streaming. */
#include "ios_rs.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <UDID>\n", argv[0]);
        return 2;
    }

    const char *udid = argv[1];
    int status = ios_runtime_init_checked();
    if (status != IOS_SUCCESS) {
        fprintf(stderr, "runtime init failed (%s)\n", ios_last_error_class());
        return 1;
    }

    IosDeviceHandle *device = NULL;
    status = ios_device_open(udid, &device);
    if (status != IOS_SUCCESS) {
        fprintf(stderr, "open device failed (%s)\n", ios_last_error_class());
        return 1;
    }
    int failed = 0;

    /* 1. Get product version */
    char *version = NULL;
    if (ios_get_product_version(device, &version) == IOS_SUCCESS) {
        printf("iOS Version: %s\n", version);
        ios_free_string_checked(&version);
    }

    /* 2. List installed applications */
    char *apps_json = NULL;
    if (ios_device_list_apps_json(device, &apps_json) == IOS_SUCCESS) {
        printf("Installed Apps: %s\n", apps_json);
        ios_free_string_checked(&apps_json);
    } else {
        fprintf(stderr, "List apps failed (%s)\n", ios_last_error_class());
        failed = 1;
    }

    /* 3. List AFC root directory */
    char *files_json = NULL;
    if (ios_device_afc_list_files_json(device, "/", &files_json) == IOS_SUCCESS) {
        printf("AFC Root Files: %s\n", files_json);
        ios_free_string_checked(&files_json);
    } else {
        fprintf(stderr, "AFC list failed (%s)\n", ios_last_error_class());
        failed = 1;
    }

    /* 4. Read file via AFC */
    uint8_t *data = NULL;
    size_t len = 0;
    if (ios_device_afc_read_file(device, "/iTunesArtwork", 65536, &data, &len) == IOS_SUCCESS) {
        printf("Read AFC file /iTunesArtwork: %zu bytes\n", len);
        ios_free_bytes_checked(&data, &len);
    } else {
        fprintf(stderr, "AFC read failed (%s)\n", ios_last_error_class());
        failed = 1;
    }

    /* 5. Read syslog messages */
    char *logs_json = NULL;
    if (ios_device_syslog_read_json(device, 5, 2000, &logs_json) == IOS_SUCCESS) {
        printf("Syslog: %s\n", logs_json);
    } else {
        fprintf(stderr, "Syslog read failed (%s), completed prefix: %s\n",
                ios_last_error_class(), logs_json ? logs_json : "[]");
        failed = 1;
    }
    if (ios_free_string_checked(&logs_json) != IOS_SUCCESS) failed = 1;

    /* Clean shutdown */
    status = ios_device_close_checked(&device);
    return status == IOS_SUCCESS && !failed ? 0 : 1;
}
