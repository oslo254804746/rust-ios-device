#include "ios_rs.h"
#include <stdio.h>

int main(void) {
    IosDevice *devices = NULL;
    size_t count = 0;
    ios_runtime_init();
    int status = ios_list_devices_checked(&devices, &count);
    if (status != IOS_SUCCESS) {
        fprintf(stderr, "device listing failed (status %d, class %s)\n",
                status, ios_last_error_class());
        return 1;
    }
    if (count == 0) puts("no device found");
    for (size_t i = 0; i < count; ++i)
        printf("%s %s\n", devices[i].udid, devices[i].connection_type);
    ios_free_devices(devices, count);
    return 0;
}
