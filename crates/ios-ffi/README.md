# ios-ffi

C FFI bindings for the
[`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device)
workspace. Builds a `cdylib` and a `staticlib` plus a public C header
(`crates/ios-ffi/include/ios_rs.h`) that wraps the `ios-core` high-level API.

## Highlights

- C-callable surface for device listing, lockdown queries, CoreDevice tunnel lifecycle and RSD metadata. Individual service clients
  are currently available through Rust and the CLI.
- RSD service ports and advertised capability identifiers available as compact
  JSON through `ios_tunnel_rsd_services_json`.
- Both shared (`libios_ffi.so` / `.dylib` / `ios_ffi.dll`) and static
  (`libios_ffi.a` / `ios_ffi.lib`) artifacts.
- Built with `ios-core` features `mdns` and `tunnel-userspace`, so userspace
  CoreDevice tunnels work without elevated privileges.
- Released as binary archives rather than a crates.io package.

`ios_tunnel_rsd_services_json` returns a compact object of the form
`{"service.name":{"port":1234,"features":["capability"]}}`. Service keys are
sorted for repeatable output, and a service with no advertised capability
metadata has an empty `features` array. The returned string is owned by the
caller and must be released with `ios_free_string()`.

## Install

This crate is **not published to crates.io**. Use one of:

### Pre-built archives (recommended)

Download `ios-ffi-<version>-<target>.{tar.gz,zip}` from the
[GitHub Releases](https://github.com/oslo254804746/rust-ios-device/releases)
page. Each archive ships the dynamic library, the static library, and the
`ios_rs.h` header. A sibling `.sha256` file is provided.

Targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

### Build from source

```sh
cargo build --release -p ios-ffi
```

Outputs are written to `target/release/`:

| Platform | Artifacts                                              |
| -------- | ------------------------------------------------------ |
| Linux    | `libios_ffi.so`, `libios_ffi.a`                        |
| macOS    | `libios_ffi.dylib`, `libios_ffi.a`                     |
| Windows  | `ios_ffi.dll`, `ios_ffi.dll.lib`, `ios_ffi.lib`        |

The public C header is at `crates/ios-ffi/include/ios_rs.h`.

The complete device-listing example is [list_devices.c](examples/list_devices.c).
It checks errors and empty lists before using entries and releases the list on
success. `python scripts/validate.py ffi` compiles/links the public header and
calls the library without a device, checking status constants, layout, NULL
outputs and safe NULL cleanup. Integer results use `IOS_SUCCESS` and the
`IOS_ERR_*` constants in the header. Non-NULL handles and strings must be freed
exactly once; clearing a caller variable does not make a freed pointer reusable.

`ios_last_error_class()` returns the stable typed category of the caller OS
thread's last int-status operation. The borrowed static NUL-terminated string
must never be modified or freed and stays valid while the library is loaded.
Read it on the same thread before another int-status operation. A successful
status clears to `none`; void initialization/cleanup, uint16_t port queries and
the getter do not replace it. The getter needs no runtime initialization.

```c
int status = ios_list_devices_checked(&devices, &count);
if (status != IOS_SUCCESS) {
    /* Inspect a stable category, with a fallback for future codes. */
    fprintf(stderr, "device listing failed (%s)\n", ios_last_error_class());
    return 1;
}
```

Original statuses, signatures and layouts stay available. Runtime/native typed
sources supply the category before conversion; lost information stays `unknown`.
`IOS_ERR_ALLOC` can also represent embedded NUL and does not prove memory
exhaustion. Category/status alone grants no safe retry, partial-result rollback
or device cleanup acknowledgement. The linked C smoke also uses an owned
malformed ListDevices peer; no pairing or device service is routed. See the
[error contract](../../docs/error-contract.md) for exact mappings and limits.

Use `ios_list_devices_checked` when the caller needs to detect unencodable
received records. An embedded NUL in a UDID or connection type returns
`IOS_ERR_CORE` and class `protocol`, clears outputs to NULL/0, and returns no
partial array. Valid and empty lists use the existing layout and
`ios_free_devices` ownership. The original `ios_list_devices` still skips such
records for compatibility. The checked call verifies C string encoding of the
received list; it does not certify completeness of discovery upstream. The
linked smoke checks both calls over five owned binary-plist replies, with
valid, unencodable and empty input.

Use `ios_tunnel_rsd_port_checked` and `ios_tunnel_userspace_port_checked` for
explicit int-status queries. Each clears a writable output to zero before
validating the handle. NULL input is `IOS_ERR_NULL`/`invalid_input`; a zero cached
port is `IOS_ERR_STATE`/`precondition`; success supplies a nonzero `uint16_t` and
clears the category to `none`. These snapshot queries need no runtime/network
operation and do not establish current proxy/RSD reachability, kernel mode,
disconnect or explicit unsupported. The original uint16_t functions keep their
zero fallback and do not replace the category. Do not infer a reason for zero
from a previous call's category. The checked symbols require a library containing
these additions; older libraries continue to provide the legacy queries.

```c
uint16_t port = 0;
int status = ios_tunnel_rsd_port_checked(tunnel, &port);
if (status != IOS_SUCCESS) {
    fprintf(stderr, "cached RSD port unavailable (%s)\n", ios_last_error_class());
    /* No connection/retry/support inference follows from this category. */
    return 1;
}
```

The handle must remain live during a query, and output storage must not overlap
it. No concurrent close or freed-pointer reuse is supported. Rust-owned metadata
fixtures cover nonzero/zero cached values and category transitions; linked C
smoke checks the new signatures, NULL handling and output clearing. Neither
fixture certifies a physical tunnel or Apple cleanup. Initialization/cleanup
status additions are described below; stronger lifecycle APIs remain pending.

Use `ios_runtime_init_checked()` to inspect the cached shared initialization
result. Ordinary IO failure returns `IOS_ERR_RUNTIME` with its typed category;
caught unwind returns `IOS_ERR_PANIC`/`unknown`. Success clears the category.
Repeated calls reuse the same result rather than retrying initialization. The
legacy void initializer keeps its ordinary category behavior and now catches
unwinds before the C boundary. Neither call suppresses a process panic hook or
recovers from abort/invalid memory. No device is opened by initialization.

Checked cleanup takes caller owner storage: `ios_device_close_checked(&handle)`,
`ios_tunnel_close_checked(&tunnel)`, `ios_free_string_checked(&text)` and
`ios_free_devices_checked(&devices, &count)`. NULL required owner storage returns
`IOS_ERR_NULL`/`invalid_input`; a NULL owned value is a successful no-op. Both
list outputs are required, and neither changes if either is NULL. Accepted
owner pointers/count are cleared before release. Handle state poisoning returns
`IOS_ERR_STATE`/`precondition` after consumption; caught unwind is
`IOS_ERR_PANIC`/`unknown`. Even on failure, consumed allocations/aliases must not
be retried, used or freed. Repeat only with the cleared owner storage.

Read/save the original status/category before checked cleanup: every int-status
operation, including a successful no-op cleanup, replaces the observation.
The borrowed static category pointer remains valid after later calls while the
library is loaded. Inspect release errors separately. Legacy void cleanup
preserves the category. The checked symbols need a library containing them.

```c
int status = ios_list_devices_checked(&devices, &count);
const char *operation_class = ios_last_error_class();
/* Use successful results here, then release even an empty owned list. */
int release_status = ios_free_devices_checked(&devices, &count);
if (status != IOS_SUCCESS) {
    fprintf(stderr, "listing failed (%s)\n", operation_class);
} else if (release_status != IOS_SUCCESS) {
    fprintf(stderr, "release failed (%s)\n", ios_last_error_class());
}
```

Use exact returned counts, library-owned strings/handles, and writable owner
storage outside the released allocation, with distinct list pointer/count
outputs. Arbitrary/dangling pointers and concurrent use/free are not validated.
Success means local release/no-op status, without task joins, Apple teardown or
proof that no other owner retains a device. Failure after consumption does not
prove complete cleanup or authorize retry. Owned host fixtures and linked C
exercise these boundaries; they do not certify physical tunnel teardown or
actual global runtime resource exhaustion. L01/A03 retain stronger resource and
asynchronous lifecycle contracts.

## Linking

The additive `IosDeviceSession` API provides bounded existing-trust queries for
`ProductType`, `ProductVersion` and `DeviceName`. It takes an explicit UDID,
`tcp://` numeric socket address or an absolute Unix socket endpoint, and
caller-loaded serialized pairing bytes of at most 1MiB. Creation decodes/copies
the borrowed inputs without initializing the runtime or opening a socket. It
does not load files, resolve DNS, pair/save records, start services or tunnels.
The three finite selector bits are defined in the public header; there is no
arbitrary key parameter. Each admitted query opens its own socket, relists the
exact UDID and connects the fresh DeviceID to lockdown before the checked
QueryType/StartSession and optional TLS exchange.

```c
IosDeviceSessionConfig config = {
    udid, udid_len, endpoint, endpoint_len, pair_bytes, pair_len, 1
};
IosDeviceSession *session = NULL;
IosOperationCancel *cancel = NULL;
IosDeviceQueryOptions options;
char *json = NULL;
int status = ios_device_session_create(&config, &session);
if (status == IOS_SUCCESS) status = ios_operation_cancel_create(&cancel);
if (status == IOS_SUCCESS) status = ios_device_query_options_default(&options);
if (status == IOS_SUCCESS) {
    status = ios_device_session_query_json(session, IOS_DEVICE_INFO_ALL,
                                           &options, cancel, &json);
}
const char *query_class = ios_last_error_class();
/* Use json on success; save status/query_class before int-status cleanup. */
ios_free_string_checked(&json);
if (session) ios_device_session_close(session, 1000);
ios_device_session_destroy_checked(&session);
ios_operation_cancel_destroy_checked(&cancel);
```

[device_info.c](examples/device_info.c) is a complete synchronous caller. The
example loads pairing bytes from stdin as caller code and clears that buffer
immediately after create; the library never owns or clears the caller buffer.
Core zeroizes the parsed host private key on its actual final Drop. Avoid
printing pairing records or credentials. Successful JSON is a library-owned
string using the existing string-free protocol. The output includes selected
values, `value_present`, the field mask and selected routing/session metadata.
Absent selected strings are `null`; present empty strings are `""`. Missing
wire `Value` has `value_present: false`, while an empty root dictionary has
`value_present: true`. Explicit remote `MissingValue` remains a typed error.
Unselected fields are omitted, including on a root query; embedded NUL in a
selected string is escaped in JSON rather than truncating the C string.

Sessions share 64 active Core leases in this linked library, with a configured
local bound of 1..4. Admission has no wait queue: local exhaustion returns
`IOS_ERR_CORE`/`conflict`, shared exhaustion `resource_limit`. Options have
checked finite native-reply, retained UTF-8, metadata and C-JSON limits. Native
reply accounting includes reply framing/declared bodies, excluding requests and
TLS handshake/record overhead. JSON is bounded before each append and excludes
its final C NUL. These limits do not bound ordinary plist parser temporary
allocations, physical CPU preemption or process RSS. Total/cleanup inputs accept
0..86400000ms; a query total of zero times out before native IO. The native
absolute total starts inside immediate lease preparation. The separate C parent
clock is minted just before prepare, after validation/runtime initialization;
it is not a shared absolute deadline beginning at the C call entrance. After
prepare and on the first `block_on` poll, cancellation/closed/deadline checks
drop an expired unpolled job and its real lease before native IO. C projection
checks the parent deadline/cancellation again before append/publication. An
already obtained native error preserves its original category over a later
output fence. Every error clears the output and returns no partial string.

Each `retain` returns an independently owned opaque C allocation sharing the
same owner/token. Give independently retained handles to separately owned C
threads. Destroying one session allocation leaves other allocations open;
destroying the last broadcasts owner close/cancellation without promising an
asynchronous lease join. Explicit `close` seals **all** session aliases,
broadcasts actual cancellation and waits for actual Core lease retirement under
the cleanup budget. Timeout keeps the session closed and its outstanding leases
intact; repeated close can succeed after they actually release. An invalid
cleanup budget does not seal it. Destroy consumes/clears only one caller owner;
repeating with the cleared NULL pointer is a successful no-op.

Query and close may run concurrently while their borrowed allocations remain
live. Before destroying an allocation, callers must stop new uses and join all
original threads/functions using **that allocation**, including retained raw
pointer aliases. A successful close acknowledges Core lease release, not the
completion of C projection, an OS-thread join, remote StopSession or Apple
cleanup. It does not authorize freeing an in-flight caller's handle, output or
options. Cancellation is a permanent, idempotent signal on the actual Core
token; it can be sent from another C thread through a separately retained token.
Token destroy does not signal cancellation, and prepared jobs keep their own
token snapshot until real Drop. Checked destroys validate NULL owner storage,
consume/clear a library-owned live value and tolerate a NULL owned value. They
cannot validate arbitrary addresses, dangling pointers or concurrent use/free.
All specified input buffers must be valid for the stated lengths; writable
outputs must not overlap inputs/opaque allocations. Old handle layouts,
statuses, signatures and enum values are unchanged.
Admission/byte bounds do not cap global CPU/RSS or the number of session/token
allocations and successful output strings still retained by callers.

Blocking query/close calls require a foreign thread outside an entered Tokio
runtime. An entered runtime is refused before query admission with
`IOS_ERR_RUNTIME`/`precondition`; valid close still seals before this refusal.
The existing shared runtime's cached ordinary initialization failure remains
`IOS_ERR_RUNTIME` with its original typed category. It has no global shutdown
API. Create/retain/token signal/destroy need no runtime or native source. Read
`ios_last_error_class()` on the exact caller thread before another int-status
call, including cleanup. Native business/IO failures keep the Core cause;
truncated native `UnexpectedEof` remains `io` rather than being guessed as a
device disconnect.

The dedicated host gate is `scripts/ffi_device_session.py`, separate from the
ordinary FFI smoke. It never starts a Rust compiler. Static C checks require no
library:

```sh
python3 scripts/ffi_device_session.py --static
python3 scripts/ffi_device_session.py --fingerprint
```

The integration agent first freezes the sources and runs the required Rust
build/tests in the shared serial build queue, separately from this runner. For
example, the explicit build action is `cargo build -p ios-ffi --release`; the
runner itself does not perform that action. Save the successful build log and
record its SHA together with the fingerprint of the frozen inputs and the
fresh dynamic library in a JSON build-evidence file with these required fields:

```json
{
  "build_result": "passed",
  "source_fingerprint": "SHA256 from the --fingerprint sha256 field",
  "library_sha256": "SHA256 of the freshly built dynamic library",
  "rust_build_command": "cargo build -p ios-ffi --release",
  "rust_toolchain": "the actual toolchain used by the integration agent",
  "build_mode": "release",
  "build_log": { "path": "/absolute/path/to/build.log", "sha256": "actual log SHA256" }
}
```

Run the linked gate as another serial validation action with a new evidence
directory:

```sh
python3 scripts/ffi_device_session.py \
  --library target/release/libios_ffi.so \
  --build-evidence /absolute/path/to/build-evidence.json \
  --output /absolute/path/to/new-c-device-evidence
```

It rejects mismatched library/source/log evidence, installs the exact hashed
header/library in its private prefix, strictly links the real C caller and
checks the actual loaded library path. Every native scenario uses the production
query path over usbmux framing, checked lockdown and, where selected, a real TLS
handshake. Original C pthreads are joined; the original Python peer then records
strict raw TCP EOF and joins before allowing the C behavior assertions. TLS
`recv() == b""`, a reset or a timeout is insufficient TCP EOF evidence. It does
not claim TLS close_notify or Apple acknowledgement. Scenarios cover all finite
single keys/root selection, missing/empty/binary-NUL output, fresh DeviceIDs,
typed native/business failures, bounded reply/UTF-8/JSON failure, timeout,
cross-thread real cancellation/close and local admission with no second source.
The no-source scenario covers invalid inputs, NULLs and repeat checked frees.
Source/library hashes are checked again after the batch; full bounded C output
and native phase/EOF/join evidence are saved even on failure. The runner uses no
background daemon, concurrent C batch or stale result file as acceptance.

This is a finite A03 host subitem. Rust compilation/tests and this linked gate
must actually pass after integration; static C success alone does not accept
it. Physical devices, other platform counterparts, actual global runtime
resource exhaustion, file/apps/log operations and the A03 main task remain
unaccepted until their own requirements and external conditions are met.

A minimal CMake snippet:

```cmake
add_library(ios_rs SHARED IMPORTED)
set_target_properties(ios_rs PROPERTIES
    IMPORTED_LOCATION   "${CMAKE_SOURCE_DIR}/vendor/ios_ffi/libios_ffi.so"
    INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_SOURCE_DIR}/vendor/ios_ffi"
)
target_link_libraries(my_target PRIVATE ios_rs)
```

On Windows you may also need to link `ws2_32`, `bcrypt`, and `userenv` when
linking against the static library. On Linux and macOS, link with `pthread`
and `dl`.

## Requirements

- Rust **1.80+** for source builds.
- usbmux on the host (usbmuxd on Linux, Apple Mobile Device Support on
  Windows, Apple device support on macOS).
- A trusted iOS device for most real-device operations.
- For CoreDevice tunnels: a compatible iOS 17+ device with pairing material
  on the host.
- On Windows: OpenSSL provided through vcpkg with the
  `x64-windows-static-md` triplet (set `VCPKG_ROOT`, `VCPKGRS_TRIPLET`,
  `OPENSSL_STATIC=1`).

## Documentation

- [Public compatibility rules](../../docs/public-contract.md) preserve existing
  signatures, layouts and ownership. Close frees a non-NULL C handle; clear the
  caller pointer before any later use or cleanup.
- Repository: <https://github.com/oslo254804746/rust-ios-device>
- Header: [`crates/ios-ffi/include/ios_rs.h`](https://github.com/oslo254804746/rust-ios-device/blob/master/crates/ios-ffi/include/ios_rs.h)
- High-level API reference: <https://docs.rs/ios-core>
- CoreDevice tunnel: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/tunnel.md>

## License

Licensed under either of Apache-2.0 or MIT at your option.
