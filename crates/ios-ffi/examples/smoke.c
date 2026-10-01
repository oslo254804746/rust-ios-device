/* Host-only ABI smoke: never opens or pairs a device. */
#include "ios_rs.h"
#include <assert.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

_Static_assert(IOS_TUN_USERSPACE == 0 && IOS_TUN_KERNEL == 1, "mode ABI");
_Static_assert(sizeof(IosTunMode) == sizeof(int), "mode layout ABI");
_Static_assert(IOS_SUCCESS == 0 && IOS_ERR_NULL == 1 && IOS_ERR_UTF8 == 2 &&
               IOS_ERR_CORE == 3 && IOS_ERR_ALLOC == 4 && IOS_ERR_STATE == 5 &&
               IOS_ERR_RUNTIME == 6 && IOS_ERR_PANIC == 7 &&
               IOS_ERR_BUFFER_TOO_SMALL == 8, "status ABI");
_Static_assert(offsetof(IosDevice, udid) == 0, "device layout");
_Static_assert(offsetof(IosDevice, device_id) == sizeof(char *), "device layout");
_Static_assert(offsetof(IosDevice, connection_type) ==
               ((sizeof(char *) + sizeof(uint32_t) + _Alignof(char *) - 1) /
                _Alignof(char *)) * _Alignof(char *), "device layout");

int main(int argc, char **argv) {
    /* Assign every public entry to its existing C signature. Strict warnings
       reject a header change; linking exercises the exported symbol names. */
    void (*runtime_fn)(void) = ios_runtime_init;
    const char *(*class_fn)(void) = ios_last_error_class;
    int (*list_fn)(IosDevice **, size_t *) = ios_list_devices;
    void (*free_devices_fn)(IosDevice *, size_t) = ios_free_devices;
    int (*open_fn)(const char *, IosDeviceHandle **) = ios_device_open;
    void (*close_device_fn)(IosDeviceHandle *) = ios_device_close;
    int (*version_fn)(const IosDeviceHandle *, char **) = ios_get_product_version;
    int (*value_fn)(const IosDeviceHandle *, const char *, char **) = ios_get_lockdown_value_json;
    void (*free_string_fn)(char *) = ios_free_string;
    int (*start_fn)(const char *, IosTunMode, IosTunnel **) = ios_start_tunnel;
    void (*close_tunnel_fn)(IosTunnel *) = ios_tunnel_close;
    int (*address_fn)(const IosTunnel *, char *, size_t) = ios_tunnel_server_address;
    uint16_t (*rsd_fn)(const IosTunnel *) = ios_tunnel_rsd_port;
    uint16_t (*proxy_fn)(const IosTunnel *) = ios_tunnel_userspace_port;
    int (*services_fn)(const IosTunnel *, char **) = ios_tunnel_rsd_services_json;
    assert(runtime_fn && list_fn && free_devices_fn && open_fn && close_device_fn &&
           version_fn && value_fn && free_string_fn && start_fn && close_tunnel_fn &&
           address_fn && rsd_fn && proxy_fn && services_fn && class_fn);
    assert(strcmp(class_fn(), "none") == 0);
    IosDevice *devices = NULL;
    size_t count = 99;
    IosDeviceHandle *device = NULL;
    IosTunnel *tunnel = NULL;
    char *text = NULL;
    char address[46] = {0};
    ios_runtime_init();
    ios_runtime_init();
    assert(strcmp(class_fn(), "none") == 0);
    if (argc == 2 && strcmp(argv[1], "--owned-list-error") == 0) {
        /* The validation runner supplies the sole owned ListDevices endpoint;
           malformed response can never route a device service. */
        assert(ios_list_devices(&devices, &count) == IOS_ERR_CORE);
        assert(devices == NULL && count == 0);
        const char *saved_class = class_fn();
        assert(strcmp(saved_class, "protocol") == 0);
        ios_free_devices(NULL, 0);
        assert(ios_tunnel_rsd_port(NULL) == 0);
        assert(strcmp(class_fn(), "protocol") == 0);
        assert(ios_list_devices(NULL, &count) == IOS_ERR_NULL && count == 0);
        assert(strcmp(class_fn(), "invalid_input") == 0);
        assert(strcmp(saved_class, "protocol") == 0);
        puts("C ABI owned error passed");
        return 0;
    }
    if (argc != 1) return 2;
    assert(ios_list_devices(NULL, &count) == IOS_ERR_NULL && count == 0);
    assert(strcmp(class_fn(), "invalid_input") == 0);
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
    assert(strcmp(class_fn(), "invalid_input") == 0);
    puts("C ABI smoke passed");
    return 0;
}
