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
 * List installed user applications as newly-allocated JSON array string.
 *
 * Array of objects: [{"bundle_id": "...", "display_name": "...", "version": "...", "app_type": "...", "path": "..."}].
 * Free the returned string with ios_free_string() or ios_free_string_checked().
 */
int ios_device_list_apps_json(const IosDeviceHandle *handle, char **json_out);

/**
 * List directory contents via AFC as newly-allocated JSON array of file names.
 *
 * Free the returned string with ios_free_string() or ios_free_string_checked().
 */
int ios_device_afc_list_files_json(const IosDeviceHandle *handle, const char *path, char **json_out);

/**
 * Read a file via AFC with an upper byte limit.
 *
 * On success, *data_out points to newly-allocated buffer of *len_out bytes.
 * Free the buffer with ios_free_bytes() or ios_free_bytes_checked().
 */
int ios_device_afc_read_file(const IosDeviceHandle *handle, const char *path, size_t max_bytes, uint8_t **data_out, size_t *len_out);

/**
 * Free a byte buffer returned by an ios-rs byte-output function.
 */
void ios_free_bytes(uint8_t *data, size_t len);

/**
 * Consume/free a byte buffer and reset caller pointers.
 */
int ios_free_bytes_checked(uint8_t **data_in_out, size_t *len_in_out);

/**
 * Read log messages from syslog service as newly-allocated JSON array of strings.
 *
 * Reads up to max_messages or until timeout_ms expires.
 * Free the returned string with ios_free_string() or ios_free_string_checked().
 */
int ios_device_syslog_read_json(const IosDeviceHandle *handle, size_t max_messages, uint64_t timeout_ms, char **json_out);

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

/* ── Finite existing-trust device information (additive API) ───────────────── */

typedef struct IosDeviceSession IosDeviceSession;
typedef struct IosOperationCancel IosOperationCancel;

#define IOS_DEVICE_INFO_PRODUCT_TYPE UINT32_C(1)
#define IOS_DEVICE_INFO_PRODUCT_VERSION UINT32_C(2)
#define IOS_DEVICE_INFO_DEVICE_NAME UINT32_C(4)
#define IOS_DEVICE_INFO_ALL UINT32_C(7)

/** Borrowed buffers, valid only during create. Lengths exclude any terminator.
 * Text is UTF-8, nonempty, contains no NUL, and is at most 4096 bytes. Endpoint
 * is explicit tcp://<numeric SocketAddr> with nonzero port, or on Unix an
 * absolute unix:///socket/path. No environment/default endpoint or DNS lookup.
 * Pair bytes are caller-loaded serialized plist, nonempty and <=1MiB. Caller
 * owns/clears its buffer; the parsed host private key is zeroized on final Drop.
 * concurrent_operations is exactly 1..4, sharing 64 active operation leases
 * across sessions in this linked library. There is no admission waiter queue.
 */
typedef struct IosDeviceSessionConfig {
    const char *udid;
    size_t udid_len;
    const char *endpoint;
    size_t endpoint_len;
    const uint8_t *pair_record;
    size_t pair_record_len;
    uint32_t concurrent_operations;
} IosDeviceSessionConfig;

/** Native reply limits count the two mux framed replies and three framed
 * lockdown replies. Requests and TLS handshake/record bytes do not count.
 * Metadata/selected UTF-8 limits apply after ordinary plist decoding, before
 * retained clones; they are not parser CPU/temporary allocation/RSS guarantees.
 * json_bytes bounds serialized C output before append, excluding the final NUL.
 * total_ms accepts 0..86400000; zero native query total gives timeout before IO.
 * reply_wire_bytes: 1..46137356; output_utf8_bytes: 1..16777216;
 * mux_frame_bytes: 16..16777216; lockdown_body_bytes: 1..4194304;
 * metadata_entries: 1..1000000; metadata_utf8_bytes: 1..16777216;
 * json_bytes: 1..100664320. Larger/zero byte limits are invalid_input.
 */
typedef struct IosDeviceQueryOptions {
    uint64_t total_ms;
    size_t reply_wire_bytes;
    size_t output_utf8_bytes;
    size_t mux_frame_bytes;
    size_t lockdown_body_bytes;
    size_t metadata_entries;
    size_t metadata_utf8_bytes;
    size_t json_bytes;
} IosDeviceQueryOptions;

/** Fill defaults: 5000ms; 1MiB native reply/mux/lockdown/metadata/JSON;
 * 64KiB retained UTF-8; 1024 metadata entries. Required writable output.
 */
int ios_device_query_options_default(IosDeviceQueryOptions *options_out);

/** Create without IO, file loading, pairing, saving, service start or tunnel.
 * Config and all supplied buffers must be valid for their exact lengths.
 * Required writable output is cleared to NULL before validation/fallible work.
 * Outputs/config/buffers must not overlap. Invalid addresses are not validated.
 * Ordinary parse/native errors use IOS_ERR_CORE and their typed category.
 */
int ios_device_session_create(const IosDeviceSessionConfig *config,
                              IosDeviceSession **session_out);

/** Create a distinct owned C allocation sharing the original session owner.
 * Borrowed handle stays live throughout; output is distinct/writable and cleared
 * before validation. Retaining a closed session does not reopen it.
 */
int ios_device_session_retain(const IosDeviceSession *session,
                              IosDeviceSession **session_out);

/** Seal ALL retained aliases, broadcast actual cancellation, await actual local
 * Core lease release under cleanup_ms (0..86400000), without freeing this handle.
 * Query/close may run concurrently on live handles; timeout retains closed state
 * and original outstanding counts. Repeated close can finish after real release.
 * Invalid budget is rejected before sealing. A valid close seals before runtime
 * refusal/failure; such failure does not acknowledge lease retirement.
 * No caller OS-thread join or remote StopSession/Apple teardown acknowledgement.
 * Blocking query/close must run on foreign threads outside an entered Tokio
 * runtime; refusal is IOS_ERR_RUNTIME/precondition. Runtime initialization IO
 * failure is IOS_ERR_RUNTIME with its original category.
 */
int ios_device_session_close(const IosDeviceSession *session, uint64_t cleanup_ms);

/** Consume ONE exclusively owned C allocation and clear writable owner storage
 * before release. NULL owner storage: IOS_ERR_NULL; NULL owned value: success.
 * Destroying one retained allocation does not close other live allocations.
 * Last allocation Drop seals/cancels, but destroy does not async join leases.
 * Caller must stop and JOIN all calls using THIS allocation before destroy;
 * raw pointer aliases to it are invalid afterwards, including on failure.
 * Other independently retained allocations can remain in use. No arbitrary,
 * dangling or concurrent-use pointer validation is supplied. For an observable
 * release receipt, explicitly close, join caller threads, then destroy owners.
 */
int ios_device_session_destroy_checked(IosDeviceSession **session_in_out);

/** Select only ProductType/ProductVersion/DeviceName with a nonempty bitset.
 * Unknown bits are invalid_input. One bit sends that finite GetValue key;
 * multiple bits request root but retain/project only selected fields.
 * Every admitted operation dials a fresh owned socket, relists exact UDID,
 * routes fresh DeviceID, and performs checked existing-trust StartSession/TLS.
 * No Pair/save/tunnel/service/file-loader/DNS/retry path is called.
 * Options, session and required cancel handle must stay live throughout call.
 * Writable output is cleared to NULL before validation; no partial string.
 * JSON includes value_present, requested_fields, values, selected device_id/
 * connection_type/product_id and enable_session_ssl. Selected absent strings
 * are null; present empty strings are ""; unselected fields are omitted. Missing
 * Value differs from an empty root dictionary. Explicit remote MissingValue
 * stays an error. JSON escaping preserves embedded NUL in device strings.
 * Free success output with ios_free_string or ios_free_string_checked.
 * Runtime initialization/validation precede the C parent clock. Native total
 * starts inside immediate lease preparation; the separate C parent deadline is
 * minted just before prepare. After prepare and on block_on's first poll, a
 * parent cancel/closed/deadline fence drops an expired unpolled job before IO.
 * Projection also checks that parent fence before append/publication. These
 * are separate cooperative clocks, not a shared C-entry absolute deadline.
 * Synchronous parsing/serialization cannot be physically preempted. Native Err
 * acquired by the prepared operation remains primary over a later C fence.
 * Close acknowledges lease release, not completion/join of a caller's entire
 * C function: JOIN caller threads before freeing their handles/output storage.
 * Admission/reply/output byte limits do not cap global CPU/RSS or the number
 * of successful output strings still retained by callers.
 * No output/options/input may overlap an opaque handle. All pointers require
 * valid ownership/lifetimes; null checks do not validate arbitrary addresses.
 */
int ios_device_session_query_json(const IosDeviceSession *session,
                                  uint32_t fields,
                                  const IosDeviceQueryOptions *options,
                                  const IosOperationCancel *cancel,
                                  char **json_out);

/** Required writable output is cleared before allocation. No runtime/IO. */
int ios_operation_cancel_create(IosOperationCancel **cancel_out);

/** Distinct owned allocation sharing the same actual cancellation token.
 * Borrowed token stays live; writable output is distinct and cleared first.
 * Give a retained allocation to each independently owned C thread.
 */
int ios_operation_cancel_retain(const IosOperationCancel *cancel,
                               IosOperationCancel **cancel_out);

/** Idempotently cancel all queries using this token's prepared snapshot.
 * Live token required. Cancellation is permanent; create a new token for reuse.
 * No remote rollback acknowledgement or synchronous caller-thread join.
 */
int ios_operation_cancel(const IosOperationCancel *cancel);

/** Consume/clear one owned allocation after all calls on it stopped/joined.
 * NULL owner storage is IOS_ERR_NULL; NULL owned value succeeds repeatedly.
 * Does NOT cancel: prepared jobs/other retained allocations keep the snapshot.
 * Never destroy an allocation concurrently being used, or reuse raw aliases.
 */
int ios_operation_cancel_destroy_checked(IosOperationCancel **cancel_in_out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* IOS_RS_H */
