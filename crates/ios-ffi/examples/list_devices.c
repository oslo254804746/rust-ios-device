#include "ios_rs.h"
#include <stdio.h>

int main(void) {
    IosDevice *devices = NULL;
    size_t count = 0;
    int status = ios_runtime_init_checked();
    if (status != IOS_SUCCESS) {
        fprintf(stderr, "runtime initialization failed (status %d, class %s)\n",
                status, ios_last_error_class());
        return 1;
    }
    status = ios_list_devices_checked(&devices, &count);
    if (status != IOS_SUCCESS) {
        fprintf(stderr, "device listing failed (status %d, class %s)\n",
                status, ios_last_error_class());
        return 1;
    }
    if (count == 0) puts("no device found");
    for (size_t i = 0; i < count; ++i)
        printf("%s %s\n", devices[i].udid, devices[i].connection_type);
    status = ios_free_devices_checked(&devices, &count);
    if (status != IOS_SUCCESS) {
        fprintf(stderr, "device-list release failed (status %d, class %s)\n",
                status, ios_last_error_class());
        /* Ownership was consumed; do not retry freeing a retained pointer. */
        return 1;
    }
    return 0;
}
