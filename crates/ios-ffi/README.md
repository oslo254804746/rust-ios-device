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

## Linking

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
