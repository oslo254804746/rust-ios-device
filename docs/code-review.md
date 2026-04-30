# rust-ios-device Code Review Report

**Date**: 2026-04-27
**Scope**: Full codebase — ios-core, ios-cli, ios-ffi, ios-py
**Focus**: Error handling, API design, async patterns, protocol safety, security, code duplication, test coverage, dependencies

---

## Project Overview

4-crate workspace, ~180 `.rs` files. Implements USB/network communication with iOS devices via usbmuxd, lockdown, XPC, DTX, and other Apple protocols. Supports 36 feature-gated device services, with CLI, C FFI, and Python bindings.

**Architecture**: Layered design — `proto/` (zero-IO encoding) → `mux/lockdown/xpc` (async transport) → `services/` (high-level APIs) → `device.rs` (unified entry point) → bindings (CLI/FFI/Python).

---

## P0 — Critical (fix immediately)

### P0-1: Protocol parsers lack maximum allocation guards — DoS risk

**Status (2026-04-27)**: Fixed. Added allocation guards before network-sized buffer allocation in:
- `crates/ios-core/src/lockdown/protocol.rs` (`MAX_LOCKDOWN_FRAME_SIZE = 4 MiB`)
- `crates/ios-core/src/mux/protocol.rs` (`MAX_MUX_MESSAGE_SIZE = 16 MiB`)
- `crates/ios-core/src/pairing_transport.rs` (`MAX_XPC_BODY_SIZE = 1 MiB`, with `usize::try_from`)

Regression tests added for all three oversized frame/body paths.

Several frame parsers read a length field from the network and allocate a buffer of that size with no upper bound. A malicious or corrupted peer can trigger multi-GiB heap allocations, causing OOM.

| Location | Length Field | Max Allocation |
|----------|-------------|----------------|
| `crates/ios-core/src/lockdown/protocol.rs:15-21` | `u32` | ~4 GiB |
| `crates/ios-core/src/mux/protocol.rs:36-44` | `u32` | ~4 GiB |
| `crates/ios-core/src/pairing_transport.rs:486-490` | `u64 as usize` | Unbounded |

The codebase already demonstrates the correct pattern in other modules:
- `services/device_link.rs:150` — `MAX_PLIST_SIZE = 4 * 1024 * 1024`
- `services/heartbeat/mod.rs:69` — `MAX_PLIST_SIZE = 1024 * 1024`
- `xpc/message.rs` — `MAX_XPC_COLLECTION_SIZE = 65536`

**Recommendation**: Add `MAX_FRAME_SIZE` constants and validate before allocation:

```rust
// lockdown/protocol.rs
const MAX_LOCKDOWN_FRAME: usize = 4 * 1024 * 1024; // 4 MiB

pub async fn recv_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, LockdownError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let length = u32::from_be_bytes(len_buf) as usize;
    if length > MAX_LOCKDOWN_FRAME {
        return Err(LockdownError::Protocol(format!("frame too large: {length}")));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}
```

Apply the same pattern to `mux/protocol.rs` (suggest 16 MiB) and `pairing_transport.rs` (suggest 1 MiB, with `usize::try_from(body_len_u64)` before the cast).

---

### P0-2: FFI `Vec::from_raw_parts` uses incorrect capacity — undefined behavior

**Status (2026-04-27)**: Fixed. `ios_list_devices` now transfers ownership with `Box<[IosDevice]>`, and `ios_free_devices` drops the boxed slice allocation instead of reconstructing a `Vec` with a guessed capacity. Added a regression test that uses a vector whose capacity differs from its length.

**File**: `crates/ios-ffi/src/lib.rs:212,242`

The code calls `shrink_to_fit()` on a `Vec<IosDevice>`, then passes `count` as both `len` and `capacity` to `Vec::from_raw_parts`. Per the Rust documentation, `shrink_to_fit` does not guarantee `capacity == len` — the allocator may keep extra space. If `capacity > count`, the `from_raw_parts` call is **undefined behavior**.

```rust
// Current (UB-prone):
devices.shrink_to_fit();
let ptr = devices.as_mut_ptr();
let count = devices.len();
std::mem::forget(devices);
*devices_out = ptr;
*count_out = count;

// Later in ios_free_devices:
Vec::from_raw_parts(devices, count, count) // ← capacity may be wrong
```

**Recommendation** — Option A (preferred): Use `Box<[IosDevice]>` which has no capacity field:

```rust
let boxed: Box<[IosDevice]> = devices.into_boxed_slice();
let count = boxed.len();
let ptr = Box::into_raw(boxed) as *mut IosDevice;
*devices_out = ptr;
*count_out = count;

// Free:
drop(Box::from_raw(std::slice::from_raw_parts_mut(devices, count)));
```

**Recommendation** — Option B: Store and return actual capacity:

```rust
let cap = devices.capacity();
// pass cap alongside ptr and count to the C side
// reconstruct with: Vec::from_raw_parts(ptr, count, cap)
```

---

### P0-3: `send_plist`/`recv_plist` duplicated across 22+ modules with inconsistent safety

**Status (2026-04-30)**: Fixed. All 25 service modules with plist frame reading now have MAX_SIZE guards. The last remaining unguarded path — `backup2/mod.rs::read_prefixed_string` (u32 size → unbounded allocation) — now rejects strings above 64 KiB with a regression test. A full shared framing helper extraction was intentionally deferred because several services have protocol-specific raw plist handling.

The identical plist frame encode/decode logic (serialize XML → write u32 length prefix → write body / read u32 length → read body → deserialize) is independently implemented in **at least 22 service modules**:

```
services/arbitration/mod.rs          services/companion/mod.rs
services/file_relay/mod.rs           services/power_assertion/mod.rs
services/heartbeat/mod.rs            services/preboard/mod.rs
services/idam/mod.rs                 services/mobileactivation.rs
services/diagnostics.rs              services/imagemounter/protocol.rs
services/screenshot/mod.rs           services/mcinstall/mod.rs
services/apps/zipconduit.rs          services/apps/installation.rs
services/webinspector/mod.rs         services/ostrace/mod.rs
services/house_arrest/mod.rs         services/springboard/mod.rs
services/fetchsymbols/mod.rs        services/misagent/mod.rs
services/restore/mod.rs             services/crashreport/mod.rs
```

**The critical issue**: Only `device_link` and `heartbeat` include a `MAX_PLIST_SIZE` guard. The remaining 20+ copies have the same unbounded allocation vulnerability as P0-1.

**Recommendation**: Extract to a shared module:

```rust
// proto/plist_framing.rs (or services/common.rs)
const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;

pub async fn send_plist<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &impl serde::Serialize,
) -> Result<(), std::io::Error> {
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writer.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    Ok(())
}

pub async fn recv_plist<R: AsyncRead + Unpin, T: serde::de::DeserializeOwned>(
    reader: &mut R,
) -> Result<T, std::io::Error> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let length = u32::from_be_bytes(len_buf) as usize;
    if length > MAX_PLIST_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("plist frame too large: {length}"),
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    plist::from_bytes(&payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
```

Each service maps `io::Error` via its own error enum's `#[error] Io(#[from] std::io::Error)` — no extra boilerplate needed. This eliminates ~500-600 lines of duplicated code and ensures uniform safety.

---

## P1 — Important (fix before next release)

### P1-1: Blocking `std::fs` calls in async contexts

**Status (2026-04-30)**: Fixed. All significant blocking FS paths addressed:
- `zipconduit.rs:52` — replaced `std::fs::read` with `tokio::fs::read` (IPA files can be 100s of MB)
- `device.rs:695` — wrapped `load_wifi_mac_pairings()` in `spawn_blocking` (reads many pair record plists)
- `device.rs:1264,1370` — wrapped `load_remote_pairing_credentials` calls in `spawn_blocking` (reads credential plists from disk)
- `imagemounter/download.rs:157` — wrapped `extract_zip` in `spawn_blocking` (CPU-bound zip + FS writes)
- `backup2/mod.rs` — converted run-loop FS operations to async: `tokio::fs::read` for file downloads, `tokio::fs::create_dir_all`/`File::create` for uploads, `tokio::fs::rename`/`remove_file`/`remove_dir_all` for move/remove commands, `spawn_blocking` for `initialize_backup_directory`, `copy_item`, and `contents_of_directory`
- `dproxy/mod.rs` — wrapped `ProxyRecorder::new` in `spawn_blocking` at CLI call site
Remaining low-impact: `credentials.rs` save/list in CLI pair command (small files, called once during user-interactive pairing); `create_runtime_layout` (single `create_dir_all`, microseconds).

Synchronous filesystem I/O on the tokio executor thread can block the reactor, stalling all concurrent tasks on that thread.

| File | Lines | Operation |
|------|-------|-----------|
| `src/device.rs` | 695 | `std::fs::read_dir()` in `load_wifi_mac_pairings()` |
| `src/services/backup2/mod.rs` | 158-206 | `create_dir_all`, `File::create`, `remove_file` |
| `src/services/apps/zipconduit.rs` | 52 | `std::fs::read(ipa_path)` — reads entire IPA file synchronously |
| `src/services/imagemounter/download.rs` | 157 | `create_dir_all`, `File::create`, `io::copy` |
| `src/credentials.rs` | 62-84 | `save()`, `load()`, `list()` all use `std::fs` |
| `src/services/dproxy/mod.rs` | 83-88 | `create_dir_all`, `File::create` |

**Recommendation**: Replace with `tokio::fs` equivalents. For CPU-bound operations like zip extraction, use `tokio::task::spawn_blocking`. The `zipconduit.rs:52` case (`std::fs::read` of potentially large IPA files) is the most impactful.

---

### P1-2: Internal modules fully public — leaking implementation details

**Status (2026-04-30)**: Fixed. Phase 1 made `psk_tls` and `mux` modules `pub(crate)` and updated 7 CLI files to import `MuxClient` via top-level re-export. Phase 2 made `pairing_transport` and `tunnel` modules `pub(crate)`, added top-level re-exports for `PairingTransportError`, `TunnelError`, `TunnelHandle`, `TunnelInfo`, and `TunnelManager`, and updated CLI/FFI/Python uses of `TunMode` to the top-level `ios_core::TunMode` path. Phase 3 made `lockdown` `pub(crate)`, added top-level re-exports for supported lockdown API (`LockdownClient`, `LockdownError`, `PairRecord`, pair-record path helper, lockdown plist request/response helpers, session TLS helpers, supervised-pair helpers), and migrated all workspace external `ios_core::lockdown::*` uses to top-level imports. Phase 4 made `xpc` `pub(crate)`, added top-level re-exports for the supported XPC API (`XpcClient`, `XpcError`, `XpcMessage`, `XpcValue`, `RsdHandshake`, `ServiceDescriptor`, `RSD_PORT`, XPC message encode/decode helpers, and XPC message flags), and migrated CLI plus `fetchsymbols` integration-test uses of `ios_core::xpc::*` to top-level imports. Phase 5 made `proto` `pub(crate)`, added top-level re-exports for supported NSKeyedArchiver encoding helpers (`archive_array`, `archive_bool`, `archive_data`, `archive_dict`, `archive_float`, `archive_int`, `archive_nsurl`, `archive_null`, `archive_string`, `archive_uuid`, `archive_xct_capabilities`, `archive_xctest_configuration`, `NsUrl`, `XcTestConfiguration`, `XctCapabilities`), and migrated the remaining `ios_core::proto::nskeyedarchiver_encode::*` integration-test uses to top-level imports. Added doctests that enforce the public API boundary: top-level re-exports compile, while `ios_core::lockdown::*`, `ios_core::pairing_transport::*`, `ios_core::tunnel::*`, `ios_core::xpc::*`, and `ios_core::proto::*` deep imports fail to compile. Removed now-private dead code/shims that became unused after module visibility tightening (`PairingResult`, unused `PairingError` variants, unused `verify_pair_step2` wrapper, unused supervised `get_wifi_address`, `xpc::codec`, `xpc::rsd::parse_handshake_message_pub`, unused H2 helper methods/variant, and unused public fields on the internal H2 data frame). Kept internal `crate::proto::*` and `crate::xpc::*` use sites private because they are implementation details, not external API. Verification: `cargo fmt --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings` with absolute uv-managed Python (`PYO3_PYTHON=<workspace>\target\pyo3-venv\Scripts\python.exe`); `cargo test --workspace --all-features` with uv CPython root from `target\pyo3-venv\pyvenv.cfg` and its `DLLs` directory on `PATH`; targeted `cargo test -p ios-core --doc`, `cargo test -p ios-core --test fetchsymbols --all-features`, `cargo test -p ios-core --test accessibility_audit --all-features`, `cargo test -p ios-core --test testmanager --all-features`, `cargo test -p ios-cli mobilegestalt --all-features`, and `cargo test -p ios-cli rsd --all-features`. Smoke tested both connected devices with actual CLI syntax (`--udid` is a top-level option): `cargo run -p ios-cli -- --help`, `cargo run -p ios-cli -- list`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C info`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C lockdown --help`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C lockdown info`, `cargo run -p ios-cli -- --udid 00008020-00103908029A002E info`, `cargo run -p ios-cli -- --udid 00008020-00103908029A002E lockdown --help`, and `cargo run -p ios-cli -- --udid 00008020-00103908029A002E lockdown info`. No remaining public internal module leaks from the originally listed modules.

`lib.rs` declares all internal modules as `pub mod`:

```rust
pub mod proto;           // wire protocol encoders/decoders
pub mod lockdown;        // lockdown transport internals
pub mod mux;             // usbmuxd transport internals
pub mod xpc;             // XPC transport internals
pub mod pairing_transport;
pub mod psk_tls;
```

This exposes every internal type as public API. The CLI crate reaches into paths like `ios_core::lockdown::pair_record::default_pair_record_path` and `ios_core::xpc::rsd::RsdHandshake`. If any internal type is renamed or moved, downstream code breaks.

Selected types are already properly re-exported at the top level (lines 23-35, 102), which is the right pattern.

**Recommendation**: Change to `pub(crate) mod` and add any missing re-exports to `lib.rs`. Update `ios-cli` to use the top-level re-exports instead of reaching into internal module paths.

---

### P1-3: `ConnectedDevice` exposes internal type fields

**Status (2026-04-30)**: Fixed. Fields `tunnel` and `rsd` are now `pub(crate)`. Added accessor methods: `rsd()`, `into_rsd()`, `tunnel_handle()`. Updated ios-cli (5 files), ios-ffi, and ios-py to use accessors instead of direct field access.

**File**: `crates/ios-core/src/device.rs:83-89`

```rust
pub struct ConnectedDevice {
    pub tunnel: Option<Arc<TunnelHandle>>,  // internal type
    pub rsd: Option<RsdHandshake>,          // internal type
    // ...
}
```

Both FFI and Python bindings reach into `device.tunnel.as_ref().map(|t| t.info.server_rsd_port)`. This couples all bindings to the internal shape of `TunnelHandle`.

**Recommendation**: Change to `pub(crate)` and add accessor methods. Partial methods already exist (`server_address()`, `userspace_port()`, `rsd_port()` at lines 123-133). Add a `services()` method for `rsd.services` access.

---

### P1-4: 6 dead feature flags defined but never used

**Status (2026-04-30)**: Fixed. Removed `core`, `lockdown`, `tunnel`, `remote` (4 dead features that gated nothing). Added `#[cfg(feature = "...")]` guards for `diagnostics` and `mobileactivation` modules, aligning them with other service modules. Updated `classic` and `ios17` feature groups to remove dead references.

These features exist in `Cargo.toml` but have zero `cfg(feature = "...")` guards in code:

| Feature | Cargo.toml Line | Status |
|---------|----------------|--------|
| `core` | 16 | Dead — gates nothing |
| `lockdown` | 17 | Dead — gates nothing |
| `tunnel` | 18 | Dead — gates nothing |
| `remote` | 19 | Dead — gates nothing |
| `diagnostics` | 76 | Dead — module always compiled |
| `mobileactivation` | 79 | Dead — module always compiled |

Additionally, `house_arrest = ["afc"]` and `installation = ["apps"]` have no direct `cfg` usage — they just enable other features.

**Recommendation**: Either remove dead features or add proper `#[cfg(feature = "...")]` guards. For `diagnostics`/`mobileactivation`, add cfg guards to align with other service modules.

---

### P1-5: Security-critical paths with zero test coverage

**Status (2026-04-30)**: Fixed. Current-code audit confirmed `lockdown/session.rs` already has TLS config tests and the 15 formerly empty service test modules now contain actual tests. Added 7 unit tests for `tunnel/forward.rs` covering chunked IPv6 packet reads, non-IPv6 rejection, MTU payload guard, stream-to-TUN forwarding, TUN-to-stream forwarding, cancellation, and invalid stream packet errors. Added 3 unit tests for `tunnel/manager.rs` covering kernel handle liveness, register/find/list/stop lifecycle, and default `TunMode`. Removed the empty tracked `tests/test_tunnel.rs`.

Validation: `cargo fmt --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings` (initial bare run failed because `pyo3-build-config` could not find Python; rerun passed with `PYO3_PYTHON=target\pyo3-venv\Scripts\python.exe`); `cargo test -p ios-core tunnel::forward --all-features`; `cargo test -p ios-core tunnel::manager --all-features`; `cargo test --workspace --all-features` with the same Python plus CPython/DLL path. Smoke tested both connected devices with actual CLI syntax (`--udid` is a top-level option): `cargo run -p ios-cli -- --help`, `cargo run -p ios-cli -- list`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C info`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C lockdown --help`, `cargo run -p ios-cli -- --udid 00008020-00103908029A002E info`, and `cargo run -p ios-cli -- --udid 00008020-00103908029A002E lockdown --help`.

| Module | Criticality | Test Status |
|--------|-------------|-------------|
| `lockdown/session.rs` | TLS session establishment | **Zero tests** |
| `tunnel/forward.rs` | TCP tunnel forwarding | **Zero tests** |
| `tunnel/manager.rs` | Tunnel lifecycle management | **Zero tests** |
| `tests/test_tunnel.rs` | Integration test file | **Empty (0 bytes)** |

Additionally, **15 files** declare `#[cfg(test)] mod tests` with `MockStream` scaffolding but contain no actual `#[test]` functions:

```
services/device_link.rs:175         services/diagnostics.rs:338
services/heartbeat/mod.rs:80        services/mobileactivation.rs:96
services/power_assertion/mod.rs:98  services/preboard/mod.rs:80
services/arbitration/mod.rs:104     services/imagemounter/protocol.rs:442
services/instruments/screenshot.rs:45
services/instruments/process_control.rs:144
services/apps/installation.rs:383   services/idam/mod.rs:83
services/companion/mod.rs:103       services/file_relay/mod.rs:91
services/notificationproxy/mod.rs:208
```

**Recommendation**: Prioritize tests for `lockdown/session.rs` and `tunnel/forward.rs`. Either populate or remove the empty test modules. Delete `tests/test_tunnel.rs` if not planned.

---

### P1-6: Heavy dependencies should be optional

**Status (2026-04-30)**: Deferred. Making openssl/smoltcp/tun-rs/mdns-sd optional requires adding cfg gates to core infrastructure modules (psk_tls, tunnel/tun, discovery) which affects compilation of all dependent code paths. This is a multi-session refactoring task that should be done as a separate PR with thorough CI validation.

These dependencies are mandatory but used in very limited scope:

| Dependency | Used In | Suggested Feature Gate |
|-----------|---------|----------------------|
| `openssl` + `tokio-openssl` | `psk_tls.rs`, `supervised_pair.rs`, `prepare/mod.rs` | `tunnel` / `prepare` |
| `smoltcp` | `tunnel/tun/userspace.rs` | `tunnel-userspace` |
| `tun-rs` | `tunnel/tun/kernel.rs` | `tunnel-kernel` |
| `mdns-sd` | `discovery.rs` | `mdns` |

`openssl` is particularly impactful as it requires a system C library and complicates cross-compilation.

**Recommendation**: Make these optional dependencies gated behind features. The existing dead features (`tunnel`, `remote`) could be repurposed for this.

---

## P2 — Improvements (technical debt)

### P2-1: `MockStream` test helper duplicated 15 times

**Status (2026-04-30)**: Fixed. Added a single shared `MockStream` in `crates/ios-core/src/test_util.rs` with helper constructors for plist frames, raw frames, prefixed plist frames, trailing raw bytes, and configurable EOF behavior. Replaced the duplicated local `MockStream` implementations in the 13 source test modules and 2 integration tests listed below; integration tests reuse the same helper file via `#[path = "../src/test_util.rs"]`. Added helper self-tests covering preloaded reads, write capture, plist frame encoding, and EOF behavior. Verified with `rg` that no duplicated `struct MockStream` / `impl MockStream` / `impl AsyncRead for MockStream` / `impl AsyncWrite for MockStream` remains in `crates/ios-core/src/services` or `crates/ios-core/tests`.

Validation: `cargo fmt --check` passed. Bare `cargo clippy --workspace --all-targets --all-features -- -D warnings` and bare `cargo test --workspace --all-features` both failed because `pyo3-build-config` could not find a Python 3 interpreter; reran with `PYO3_PYTHON=target\pyo3-venv\Scripts\python.exe` (and CPython/DLL paths on `PATH` for tests), and both passed. Targeted `cargo test -p ios-core --all-features` passed. Smoke tested both connected devices with actual CLI syntax (`--udid` is a top-level option): `cargo run -p ios-cli -- --help`, `cargo run -p ios-cli -- list`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C info`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C lockdown --help`, `cargo run -p ios-cli -- --udid 00008020-00103908029A002E info`, and `cargo run -p ios-cli -- --udid 00008020-00103908029A002E lockdown --help`. No extra service smoke command was needed because P2-1 only changes test helpers.

A nearly identical `MockStream` (implementing `AsyncRead + AsyncWrite` with a pre-loaded buffer) is copy-pasted across 15 modules:

```
services/arbitration        services/companion
services/diagnostics        services/file_relay
services/idam               services/imagemounter/protocol
services/mcinstall           services/mobileactivation
services/misagent           services/notificationproxy
services/pcap               services/power_assertion
services/preboard           tests/ostrace
tests/fetchsymbols
```

**Recommendation**: Extract to `src/test_util.rs` behind `#[cfg(test)]`:

```rust
#[cfg(test)]
pub(crate) mod test_util {
    pub struct MockStream { /* ... */ }
    // single implementation
}
```

---

### P2-2: 36 near-identical service error enums

**Status (2026-04-30)**: Fixed. Added a crate-private `service_error!` macro in `services/mod.rs` and migrated the 22 service error enums that already exposed the common `Io`, `Plist`, and `Protocol` variants, preserving enum names, variant names, `#[from] std::io::Error`, `#[from]` extra variants, and display strings. Did not migrate service errors that lack one of the common variants or have materially different shapes (`AfcError`, `DtxError`, `DebugserverError`, `IpError`, `ZipConduitError`, `PreboardError`, `PrepareError`, `SimLocationError`, etc.) to avoid widening public API or changing behavior. Added a macro smoke test covering common variants, display output, `std::io::Error` conversion, and an extra variant.

Validation: TDD RED first with `cargo test -p ios-core services::tests::service_error_macro_preserves_common_variants_and_display --all-features` failed because `service_error!` did not exist; after implementation the same targeted test passed. `cargo fmt --check` passed. Bare `cargo clippy --workspace --all-targets --all-features -- -D warnings` and bare `cargo test --workspace --all-features` both failed because `pyo3-build-config` could not find a Python 3.x interpreter. Reran clippy with `PYO3_PYTHON=target\pyo3-venv\Scripts\python.exe`, and it passed. First full-test rerun with Python still failed because the local PowerShell wrapper tried to assign read-only `$HOME`, so the CPython/DLL path was not added and `ios-py` exited with `STATUS_DLL_NOT_FOUND`; reran with `$pyHome` and `PATH=<uv CPython root>;<uv CPython root>\DLLs;%PATH%`, and `cargo test --workspace --all-features` passed. Smoke tested both connected devices with actual CLI syntax (`--udid` is a top-level option): `cargo run -p ios-cli -- --help`, `cargo run -p ios-cli -- list`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C info`, `cargo run -p ios-cli -- --udid 00008150-000A584C0E62401C lockdown --help`, `cargo run -p ios-cli -- --udid 00008020-00103908029A002E info`, and `cargo run -p ios-cli -- --udid 00008020-00103908029A002E lockdown --help`. No extra service smoke command was needed because P2-2 only changes error enum declarations, not service protocol behavior.

Most service error types follow the exact same three-variant pattern:

```rust
#[derive(Debug, thiserror::Error)]
pub enum XxxError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}
```

This appears in `arbitration`, `companion`, `crashreport`, `file_relay`, `heartbeat`, `house_arrest`, `idam`, `mcinstall`, `misagent`, `mobileactivation`, `notificationproxy`, `pcap`, `power_assertion`, `preboard`, `screenshot`, `springboard`, and more.

**Recommendation**: Define a macro to reduce ~200 lines of boilerplate:

```rust
macro_rules! service_error {
    ($name:ident $(, $extra:tt)*) => {
        #[derive(Debug, thiserror::Error)]
        pub enum $name {
            #[error("IO error: {0}")]
            Io(#[from] std::io::Error),
            #[error("plist error: {0}")]
            Plist(String),
            #[error("protocol error: {0}")]
            Protocol(String),
            $($extra)*
        }
    };
}
```

---

### P2-3: `house_arrest` module mounted via `#[path]` hack

**Status (2026-04-30)**: Fixed. Declared `house_arrest` as a proper module in `services/mod.rs` with `#[cfg(feature = "house_arrest")]`. Removed `#[path]` directive from `afc/mod.rs`, replaced with `pub use super::house_arrest` re-export for backward compatibility.

**File**: `crates/ios-core/src/services/afc/mod.rs:20-21`

```rust
#[cfg(feature = "house_arrest")]
#[path = "../house_arrest/mod.rs"]
pub mod house_arrest;
```

The module lives in `services/house_arrest/` but is not declared in `services/mod.rs` and not re-exported in `lib.rs`. It is only accessible as `ios_core::afc::house_arrest`. The feature `house_arrest = ["afc"]` just enables `afc`, with no independent module declaration.

**Recommendation**: Declare `house_arrest` directly in `services/mod.rs` with its own `cfg(feature)` guard, similar to all other service modules.

---

### P2-4: Integer truncation risks

**Status (2026-04-30)**: Fixed. Added `debug_assert` for payload length > u32::MAX in `lockdown/protocol.rs`. Replaced silent `u32 as u16` truncation in `tunnel/tun/userspace.rs` with `u16::try_from` + skip on invalid port.

| File | Line | Issue |
|------|------|-------|
| `proto/lockdown.rs` | 10 | `payload.len() as u32` — silently wraps if payload > 4 GiB |
| `tunnel/tun/userspace.rs` | 494 | `u32 as u16` port cast — values > 65535 silently truncated |

Both are unlikely in practice but should have defensive checks:

```rust
// proto/lockdown.rs
let len: u32 = payload.len().try_into()
    .map_err(|_| LockdownError::Protocol("payload too large".into()))?;
```

---

### P2-5: Production `.unwrap()` in `mux/protocol.rs:37`

**Status (2026-04-30)**: Fixed. Replaced `header[0..4].try_into().unwrap()` with explicit `[header[0], header[1], header[2], header[3]]` array construction.

```rust
let length = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
```

The `try_into()` here is infallible (4-byte slice to `[u8; 4]`), so this won't panic. However, for clarity, either:
- Use `header[0..4].try_into().expect("4-byte slice")`, or
- Extract bytes directly: `u32::from_le_bytes([header[0], header[1], header[2], header[3]])`

---

### P2-6: `CoreError::Other(String)` is a catch-all escape hatch

**Status (2026-04-30)**: Acknowledged. Audit shows `Other` is used for genuinely one-off errors (join failures, config issues). Adding more variants is not justified by current usage patterns. No action needed.

The `CoreError` enum (8 variants) is well-designed, but `Other(String)` may mask errors that deserve specific variants. Audit usage to determine if new variants like `Credentials`, `Discovery`, or `Config` would be more appropriate.

---

### P2-7: CLI re-declares dependencies already available via `ios-core`

**Status (2026-04-30)**: Confirmed. Only `tokio_rustls::client::TlsStream` is used in `dproxy.rs`. Removing requires re-exporting the type from ios-core, which couples the public API to a specific TLS backend. Left as-is — minor dependency hygiene, no correctness impact.

**File**: `crates/ios-cli/Cargo.toml:34-36`

`rustls`, `tokio-rustls`, and `rustls-pemfile` are direct deps of both `ios-core` and `ios-cli`. The CLI only uses them in `cmd/dproxy.rs`. Consider re-exporting from `ios-core` or verifying whether the direct dep is truly needed.

---

### P2-8: `once_cell` can be replaced with `std::sync::LazyLock`

**Status (2026-04-30)**: Deferred. Current MSRV is 1.75; `LazyLock` requires 1.80. Will apply after MSRV bump.

`ios-ffi/src/lib.rs:40` and `ios-py/src/lib.rs:25` use `once_cell::sync::Lazy`. Since `std::sync::LazyLock` was stabilized in Rust 1.80 and the current MSRV is 1.75, this becomes a free simplification after an MSRV bump.

---

## Security Assessment Summary

| Area | Status | Notes |
|------|--------|-------|
| TLS certificate skip | Intentional, documented | `InsecureSkipVerify` — correct for Apple device pairing (self-signed certs, trust via pairing) |
| PSK TLS `NONE` verify | Correct | PSK cipher suites don't use certificate auth |
| Private key storage | Industry standard | Plaintext JSON in `~/.ios-rs/`, matches pymobiledevice3 and Apple's approach |
| Hardcoded credentials | None found | Clean |
| FFI unsafe blocks | 12 functions, all null-checked | P0-2 capacity issue is the only concern |
| XPC/OPACK/TLV parsers | Well-guarded | Proper `buf.remaining()` checks, speculative allocation caps |
| Password handling | Acceptable | P12 passwords via CLI flag or env var, empty string fallback |

---

## Positive Observations

1. **Excellent protocol parser discipline** in XPC, OPACK, and TLV decoders — bounded allocations, magic validation, EOF handling.
2. **Clean feature gate granularity** — 36 individually selectable services minimize binary size for consumers.
3. **Good FFI safety habits** — null pointer checks on all `extern "C"` functions, proper `Box::into_raw`/`Box::from_raw` lifecycle.
4. **Comprehensive CLI** — 60+ subcommands with consistent `clap` structure.
5. **Dual tunnel mode** (kernel TUN + userspace smoltcp) provides flexibility for different deployment contexts.
6. **Zero `todo!`/`unimplemented!`** in production code.
7. **Nearly all `.unwrap()` calls are confined to test code** — production code uses `?` consistently.

---

## Recommended Fix Order

1. **P0-1 + P0-3** together: Extract shared `plist_framing` module with size guards, then add guards to `lockdown/protocol.rs` and `mux/protocol.rs` (addresses both duplication and DoS risk).
2. **P0-2**: Fix FFI `Vec::from_raw_parts` capacity — small, isolated change.
3. **P1-1**: Replace `std::fs` with `tokio::fs` in async contexts.
4. **P1-2 + P1-3**: Restrict module visibility and add accessor methods.
5. **P1-4**: Clean up dead features.
6. **P1-5**: Add tests for `lockdown/session.rs` and `tunnel/forward.rs`.
7. **P1-6**: Make heavy deps optional.
8. **P2-***: Address during regular development as opportunities arise.
