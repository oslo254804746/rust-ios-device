# ios-ffi

C FFI bindings for the
[`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device)
workspace. Builds a `cdylib` and a `staticlib` plus a public C header
(`crates/ios-ffi/include/ios_rs.h`) that wraps the `ios-core` high-level API.

## Highlights

- C-callable surface for device listing, lockdown queries, pairing/service
  access, and CoreDevice tunnel lifecycle.
- Both shared (`libios_ffi.so` / `.dylib` / `ios_ffi.dll`) and static
  (`libios_ffi.a` / `ios_ffi.lib`) artifacts.
- Built with `ios-core` features `mdns` and `tunnel-userspace`, so userspace
  CoreDevice tunnels work without elevated privileges.
- Released as binary archives rather than a crates.io package.

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

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- Header: [`crates/ios-ffi/include/ios_rs.h`](https://github.com/oslo254804746/rust-ios-device/blob/master/crates/ios-ffi/include/ios_rs.h)
- High-level API reference: <https://docs.rs/ios-core>
- CoreDevice tunnel: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/tunnel.md>

## License

Licensed under either of Apache-2.0 or MIT at your option.

