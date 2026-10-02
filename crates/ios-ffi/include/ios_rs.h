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

/* Status values returned by int functions; these match the Rust ABI. */
#define IOS_SUCCESS 0
#define IOS_ERR_NULL 1
#define IOS_ERR_UTF8 2
#define IOS_ERR_CORE 3
#define IOS_ERR_ALLOC 4
#define IOS_ERR_STATE 5
#define IOS_ERR_RUNTIME 6
#define IOS_ERR_PANIC 7
#define IOS_ERR_BUFFER_TOO_SMALL 8

/* ── Lifecycle ──────────────────────────────────────────────────────────────── */

/**
 * Initialize the ios-rs async runtime.
 * Initialize before device/tunnel operations (idempotent). The error-class
 * getter does not require runtime initialization.
 */
void ios_runtime_init(void);

/**
 * Initialize the shared runtime with int status/caller-thread category.
 * Success clears the category; cached ordinary IO failure is IOS_ERR_RUNTIME
 * with its typed category; a caught unwind is IOS_ERR_PANIC/unknown. Repeated
 * calls reuse the initialization result without retrying it. No device is opened.
 * Does not suppress a process panic hook or recover from abort/invalid memory.
 * The original void initializer reports no result and preserves the category.
 */
int ios_runtime_init_checked(void);

/**
 * Stable typed category of this caller OS thread's last int-status operation.
 *
 * Returns a borrowed static NUL-terminated string, valid while the library is
 * loaded. Never free or modify it. "none" means no int-status operation on this
 * thread yet, or its last one succeeded. Every int-status call replaces that
 * thread's category; runtime init, void cleanup, uint16_t port queries and this
 * getter do not change it. Read it on the same thread after the operation.
 *
 * Categories: timeout, cancelled, disconnected, service_missing,
 * permission_denied, protocol, invalid_input, unsupported, device_not_found,
 * not_found, resource_limit, conflict, precondition, service_failure, io,
 * unknown. Retain a fallback for future codes. Native typed sources are
 * classified before status conversion; lost/unrecognized sources stay unknown.
 * A category alone does not identify a disconnected object, grant safe retry
 * or acknowledge device cleanup. Existing numeric status values are unchanged.
 */
const char *ios_last_error_class(void);

/* ── Device listing ─────────────────────────────────────────────────────────── */

/**
 * List all connected iOS devices.
 *
 * On success: *devices_out points to a heap-allocated IosDevice array of
 * *count_out elements, returns 0.
 * On error: returns non-zero; non-null outputs are cleared to NULL/0.
 * Historical behavior skips records with an embedded NUL in either string.
 * Use ios_list_devices_checked to detect these encoding failures.
 *
 * Free with: ios_free_devices(*devices_out, *count_out);
 */
int ios_list_devices(IosDevice **devices_out, size_t *count_out);

/**
 * List received device records without skipping C encoding failures.
 *
 * If a received UDID or connection type contains an embedded NUL, returns
 * IOS_ERR_CORE with error class "protocol", and clears outputs to NULL/0.
 * No partial array is returned. Successful empty input remains a valid empty
 * list. Free successful results with ios_free_devices, using the same layout.
 * This checks C projection, not completeness of upstream discovery.
 *
 * ios_list_devices retains its historical skip behavior for compatibility;
 * callers requiring detectable encoding failures should use this entry.
 */
int ios_list_devices_checked(IosDevice **devices_out, size_t *count_out);

/**
 * Free a device list returned by either listing function, with its returned count.
 */
void ios_free_devices(IosDevice *devices, size_t count);

/**
 * Consume/free a library-owned device list and clear its caller pointer/count.
 * Both owner pointers are required: if either is NULL, returns IOS_ERR_NULL/
 * invalid_input without modifying either output. Otherwise, clears both before
 * release, including on a caught unwind. A NULL list is a successful no-op,
 * resetting count to 0. A non-NULL list requires its exact returned count.
 * Owner storage must be writable, distinct and non-overlapping with the list.
 * Failure after consumption does not authorize retry or prove complete cleanup.
 * Do not concurrently use/free it or subsequently use/free retained aliases.
 */
int ios_free_devices_checked(IosDevice **devices_in_out, size_t *count_in_out);

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
 * Consume/close a device handle and clear the caller pointer before release.
 * NULL owner storage is IOS_ERR_NULL/invalid_input; a NULL owned handle is a
 * successful no-op. Poisoned state is IOS_ERR_STATE/precondition after consumption;
 * caught unwind is IOS_ERR_PANIC/unknown. Success clears the caller-thread category.
 * No consumed handle may be retried, including on failure. Local drop does not
 * acknowledge Apple cleanup; other owners can retain the ConnectedDevice.
 * Storage must be writable, holding NULL or an exclusively owned live handle
 * returned by ios_device_open, without overlapping it. No concurrent use/close
 * or retained-alias reuse.
 */
int ios_device_close_checked(IosDeviceHandle **handle_in_out);

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
 * Free a UTF-8 string returned by an ios-rs string-output function.
 */
void ios_free_string(char *s);

/**
 * Consume/free a library-owned string and clear the caller pointer before release.
 * NULL owner storage is IOS_ERR_NULL/invalid_input; a NULL string succeeds as a
 * no-op. Caught unwind is IOS_ERR_PANIC/unknown with the pointer already cleared.
 * Storage must be writable, holding NULL or a live string returned by this library.
 * It must not overlap the string allocation.
 * Do not modify its terminator, concurrently use/free it or reuse retained aliases.
 * Failure after consumption supplies no safe retry or completed-cleanup guarantee.
 */
int ios_free_string_checked(char **string_in_out);

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
 * Drops this local owner; does not join tasks or acknowledge device/Apple cleanup.
 * Clear the caller pointer after release. No status/category is reported.
 */
void ios_tunnel_close(IosTunnel *tunnel);

/**
 * Consume/close a tunnel and clear its caller pointer before release. Status and
 * ownership rules match ios_device_close_checked. The non-NULL handle must be
 * exclusively owned and returned by ios_start_tunnel. Local drop may request
 * cancellation; no task join, orderly Apple cleanup or failure retry is promised.
 */
int ios_tunnel_close_checked(IosTunnel **tunnel_in_out);

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
 * Use the RemoteXPC/RSD transport on [server_address]:rsd_port to enumerate services.
 * Returns 0 if not available or tunnel is NULL; does not change the last category.
 * Use ios_tunnel_rsd_port_checked for an explicit status/category.
 */
uint16_t ios_tunnel_rsd_port(const IosTunnel *tunnel);

/**
 * Get the cached RSD port with an int status and caller-thread error category.
 *
 * A non-NULL port_out is cleared to 0 before checking tunnel. NULL inputs return
 * IOS_ERR_NULL/invalid_input; a zero cached port returns IOS_ERR_STATE/precondition.
 * Success writes a nonzero port and clears the category to "none". No runtime or
 * network operation is required. A snapshot does not prove current reachability,
 * disconnect, device support, or successful RSD probing. Read the category on the
 * same OS thread before another int-status call. Legacy uint16_t calls are unchanged.
 *
 * tunnel must be NULL or a live handle; port_out must be NULL or writable uint16_t
 * storage that does not overlap it. Do not concurrently close the handle.
 */
int ios_tunnel_rsd_port_checked(const IosTunnel *tunnel, uint16_t *port_out);

/**
 * Get the local userspace proxy port (IOS_TUN_USERSPACE mode only).
 *
 * Protocol: TCP connect to 127.0.0.1:<port>, then send:
 *   - 16 bytes: device IPv6 address (binary, network byte order)
 *   - 4 bytes:  target service port as little-endian uint32
 * After that, the socket is a transparent TCP connection to that service.
 *
 * Returns 0 in kernel mode, if unavailable or if tunnel is NULL; does not change
 * the last category. Use ios_tunnel_userspace_port_checked for an explicit status.
 */
uint16_t ios_tunnel_userspace_port(const IosTunnel *tunnel);

/**
 * Get the cached userspace proxy port with the same output/status/category rules
 * as ios_tunnel_rsd_port_checked. A zero cached port is IOS_ERR_STATE/precondition;
 * it does not establish kernel mode, disconnect or explicit device unsupported.
 * A nonzero snapshot does not certify that a proxy is currently accepting calls.
 * The same live-handle, output-storage and concurrent-close restrictions apply.
 */
int ios_tunnel_userspace_port_checked(const IosTunnel *tunnel, uint16_t *port_out);

/**
 * Get the RSD service directory as a newly-allocated compact JSON object.
 *
 * The object maps service names to {"port": number, "features": string[]}.
 * An empty feature array means the service did not advertise capability
 * metadata; it does not mean every operation is unsupported.
 *
 * Free *json_out with ios_free_string().
 */
int ios_tunnel_rsd_services_json(const IosTunnel *tunnel, char **json_out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* IOS_RS_H */
