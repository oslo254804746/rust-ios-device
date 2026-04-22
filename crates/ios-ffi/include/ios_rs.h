/* ios_rs.h – iOS device/tunnel C FFI
 *
 * Exposes device discovery, sync lockdown queries, and tunnel functionality.
 * Once the tunnel is up, use standard C networking to communicate with the device.
 *
 * Build:
 *   cargo build -p ios-ffi --release
 *   # Linux/macOS: target/release/libios_ffi.a
 *   # Windows:     target/release/ios_ffi.lib
 */
#ifndef IOS_RS_H
#define IOS_RS_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Types ──────────────────────────────────────────────────────────────────── */

/** Connected iOS device. Freed by ios_free_devices(). */
typedef struct IosDevice {
    /** Null-terminated UDID. */
    char    *udid;
    uint32_t device_id;
    /** Null-terminated connection type: "USB" or "Network". */
    char    *connection_type;
} IosDevice;

/** Tunnel mode. */
typedef enum IosTunMode {
    /** Userspace smoltcp tunnel – no root required. Recommended. */
    IOS_TUN_USERSPACE = 0,
    /** Kernel TUN – requires root/admin. */
    IOS_TUN_KERNEL    = 1,
} IosTunMode;

/** Opaque tunnel handle. */
typedef struct IosTunnel IosTunnel;

/** Opaque non-tunnel device handle. */
typedef struct IosDeviceHandle IosDeviceHandle;

/* ── Lifecycle ──────────────────────────────────────────────────────────────── */

/**
 * Initialize the ios-rs async runtime.
 * Must be called once before any other function (idempotent).
 */
void ios_runtime_init(void);

/* ── Device listing ─────────────────────────────────────────────────────────── */

/**
 * List all connected iOS devices.
 *
 * On success: *devices_out points to a heap-allocated IosDevice array of
 * *count_out elements, returns 0.
 * On error: returns non-zero; *devices_out and *count_out are unchanged.
 *
 * Free with: ios_free_devices(*devices_out, *count_out);
 */
int ios_list_devices(IosDevice **devices_out, size_t *count_out);

/**
 * Free a device list returned by ios_list_devices().
 */
void ios_free_devices(IosDevice *devices, size_t count);

/* ── Device handle ─────────────────────────────────────────────────────────── */

/**
 * Open a non-tunnel device handle for synchronous lockdown queries.
 *
 * The handle uses the existing pairing records but does not create a tunnel.
 * Free the handle with ios_device_close().
 */
int ios_device_open(const char *udid, IosDeviceHandle **out);

/**
 * Close and free a handle returned by ios_device_open().
 */
void ios_device_close(IosDeviceHandle *handle);

/**
 * Get the device product version as a newly-allocated UTF-8 string.
 *
 * Free the returned string with ios_free_string().
 */
int ios_get_product_version(const IosDeviceHandle *handle, char **version_out);

/**
 * Get a lockdown value as compact JSON in a newly-allocated UTF-8 string.
 *
 * Pass key = NULL to fetch the full lockdown dictionary.
 * Free the returned string with ios_free_string().
 */
int ios_get_lockdown_value_json(const IosDeviceHandle *handle, const char *key, char **json_out);

/**
 * Free a UTF-8 string returned by ios_get_product_version() or
 * ios_get_lockdown_value_json().
 */
void ios_free_string(char *s);

/* ── Tunnel ─────────────────────────────────────────────────────────────────── */

/**
 * Start a CDTunnel to the device identified by `udid`.
 *
 * udid – null-terminated UDID from IosDevice.udid
 * mode – IOS_TUN_USERSPACE (recommended) or IOS_TUN_KERNEL
 * out  – receives a newly-allocated IosTunnel* on success
 *
 * Returns 0 on success, non-zero on error.
 * Free the tunnel with ios_tunnel_close(*out).
 */
int ios_start_tunnel(const char *udid, IosTunMode mode, IosTunnel **out);

/**
 * Close and free a tunnel.
 * Cancels the underlying tunnel task and releases all resources.
 */
void ios_tunnel_close(IosTunnel *tunnel);

/**
 * Get the device's tunnel IPv6 address (null-terminated string).
 *
 * buf     – output buffer, must be at least 46 bytes
 * buf_len – size of buf
 *
 * Returns 0 on success.
 */
int ios_tunnel_server_address(const IosTunnel *tunnel, char *buf, size_t buf_len);

/**
 * Get the RSD port for iOS 17+ Remote Service Discovery.
 *
 * Connect to [server_address]:rsd_port with TLS to enumerate services.
 * Returns 0 if not available.
 */
uint16_t ios_tunnel_rsd_port(const IosTunnel *tunnel);

/**
 * Get the local userspace proxy port (IOS_TUN_USERSPACE mode only).
 *
 * Protocol: TCP connect to 127.0.0.1:<port>, then send:
 *   - 16 bytes: device IPv6 address (binary, network byte order)
 *   - 4 bytes:  target service port as little-endian uint32
 * After that, the socket is a transparent TCP connection to that service.
 *
 * Returns 0 in kernel mode or if unavailable.
 */
uint16_t ios_tunnel_userspace_port(const IosTunnel *tunnel);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* IOS_RS_H */
