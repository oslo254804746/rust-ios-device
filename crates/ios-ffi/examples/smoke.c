/* Host-only ABI smoke: never opens or pairs a device. */
#include "ios_rs.h"
#include <assert.h>
#include <stddef.h>
#include <stdio.h>

_Static_assert(IOS_TUN_USERSPACE == 0 && IOS_TUN_KERNEL == 1, "mode ABI");
_Static_assert(offsetof(IosDevice, udid) == 0, "device layout");
_Static_assert(offsetof(IosDevice, device_id) == sizeof(char *), "device layout");
_Static_assert(offsetof(IosDevice, connection_type) ==
               ((sizeof(char *) + sizeof(uint32_t) + _Alignof(char *) - 1) /
                _Alignof(char *)) * _Alignof(char *), "device layout");

int main(void) {
    IosDevice *devices = NULL;
    size_t count = 99;
    IosDeviceHandle *device = NULL;
    IosTunnel *tunnel = NULL;
    char *text = NULL;
    char address[46] = {0};
    ios_runtime_init();
    ios_runtime_init();
    assert(ios_list_devices(NULL, &count) == IOS_ERR_NULL && count == 0);
    assert(ios_list_devices(&devices, NULL) == IOS_ERR_NULL && devices == NULL);
    assert(ios_device_open(NULL, &device) == IOS_ERR_NULL && device == NULL);
    assert(ios_device_open("\xff", &device) == IOS_ERR_UTF8 && device == NULL);
    assert(ios_device_open("synthetic-ffi-credential-UDID\xff", &device) == IOS_ERR_UTF8 && device == NULL);
    assert(ios_start_tunnel("synthetic-ffi-credential-UDID\xff", IOS_TUN_USERSPACE, &tunnel) == IOS_ERR_UTF8 && tunnel == NULL);
    assert(ios_start_tunnel(NULL, IOS_TUN_USERSPACE, &tunnel) == IOS_ERR_NULL);
    assert(ios_get_product_version(NULL, &text) == IOS_ERR_NULL && text == NULL);
    assert(ios_get_lockdown_value_json(NULL, NULL, &text) == IOS_ERR_NULL);
    assert(ios_tunnel_rsd_services_json(NULL, &text) == IOS_ERR_NULL);
    assert(ios_tunnel_server_address(NULL, address, sizeof(address)) == IOS_ERR_NULL);
    assert(ios_tunnel_rsd_port(NULL) == 0);
    assert(ios_tunnel_userspace_port(NULL) == 0);
    ios_free_devices(NULL, 0);
    ios_free_string(NULL);
    ios_device_close(NULL);
    ios_tunnel_close(NULL);
    puts("C ABI smoke passed");
    return 0;
}
