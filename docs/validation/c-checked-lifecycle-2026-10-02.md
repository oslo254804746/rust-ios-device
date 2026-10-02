# A02 checked C initialization and consuming cleanup — 2026-10-02

Implementation `5c0649a5b077db5ba78a34fdba8fb2b0e07cccfe`, ten related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[FFI usage](../../crates/ios-ffi/README.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Five additive entries report int statuses: shared runtime initialization and
device/tunnel/string/list cleanup. Runtime success clears the caller-thread
category; a cached ordinary native IO initialization failure is IOS_ERR_RUNTIME
with its original typed category. Repeated calls reuse the result without retry.
Caught unwind is IOS_ERR_PANIC/unknown. The original void initializer preserves
ordinary behavior/category and now catches unwinds before the C boundary; this
does not suppress a process panic hook or recover from abort/invalid memory.
No device is opened by initialization. The actual global initializer panic and
runtime resource-exhaustion paths were not injected.

Checked cleanup takes caller owner storage. NULL required storage is
IOS_ERR_NULL/invalid_input; a NULL owned resource succeeds as a no-op. Accepted
owner pointers, and the list count, are cleared before release. List cleanup
requires both outputs: if either is NULL, neither changes. A NULL list succeeds
and clears the count; a non-null list needs its exact returned count, including
an allocated empty list. Library allocation/free boundaries remain unchanged.

Poisoned handle state yields IOS_ERR_STATE/precondition after consumption and
local drop. A caught release unwind yields IOS_ERR_PANIC/unknown with the owner
already cleared. Failure cannot authorize retry through a consumed pointer or
saved alias. Repeated cleanup is safe only through cleared owner storage. Success
is a local release/no-op status, not task join, completed cancellation, Apple
teardown or proof that no other owner retains a ConnectedDevice. Failure after
consumption does not prove that every resource was released. Concurrent use/
close, arbitrary/dangling pointers, overlapping owner storage, wrong counts and
modified string terminators are outside the safety contract.

Each checked int call replaces the category, including successful no-op cleanup.
Save the original operation status/category before cleanup and inspect release
failure separately. Saved borrowed static category codes survive later calls
while the library is loaded. Original void cleanup and uint16_t queries preserve
the category. Legacy and checked cleanup share release routines, without changes
to the original signatures, status values, error enums or data layouts. New
entries require a library providing the additive symbols. The C listing example
and crate quick start now check initialization and consuming list release.

| Check | Result |
| --- | --- |
| `cargo test -p ios-ffi --all-features --locked` | 29 PASS, zero FAIL/ignored; `/tmp/ios-a02-c-lifecycle-host.log`, terminal exit 0 |
| `cargo clippy -p ios-ffi --all-targets --all-features --locked -- -D warnings` | PASS; `/tmp/ios-a02-c-lifecycle-clippy.log`, terminal exit 0 |
| `python3 scripts/validate.py ffi` | Strict C header/link and listing example, ABI/raw capture, owned malformed and checked ListDevices/consuming release PASS; `/tmp/ios-a02-c-lifecycle-abi.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-c-lifecycle-full.log`, terminal exit 0 |
| Source/header/config review | All 23 exported function names appear in the public header and cbindgen include list. Name coverage is not generator/ABI certification; actual strict C linking checks types/symbols on Linux. Original 14 signatures and four earlier additions remain |

Six new host tests cover the following boundaries. Owned cached IO results test
the exact runtime projection used by the exported initializer: native timeout,
permission and generic IO classifications, retained original result/private text,
repeated failure and no successful operation. They do not force the real global
runtime to fail. Actual exported initialization tests repeated success, saved code
lifetime and legacy category preservation; an owned successful runtime retains
reference identity. Actual exported cleanup tests NULL versus empty owners,
library-allocated list/string consumption, unchanged valid outputs on rejected
list owner storage, allocated empty-list freeing and repeated cleared no-ops.

Allocated device/tunnel fixtures exercise actual exported close functions in
normal and poisoned states, pointer clearing, category replacement and saved
static lifetime. These handles have no ConnectedDevice owner and do not execute
physical teardown. The shared consuming helper uses a counted owned destructor:
the owner is already NULL before release, state failure or destructor unwind
results in exactly one observed drop, and a later empty-owner release invokes no
releaser. This is helper ownership/unwind evidence, not a panicking actual device
destructor fixture. Existing caller-thread isolation, source and legacy tests pass.

Strict linked C checks all five new signatures and actual initializer/NULL/no-op
calls. Its five owned ListDevices replies still cover malformed projection,
valid legacy/checked arrays and an empty allocation; valid checked output now
tests rejected owner preservation and consuming free/NULL-count reset/repeated
no-op. The valid legacy array still uses the old free function. No device service
is routed. C stdout/raw-capture markers and fixture byte counts remain unchanged.

There was no product/test failure in targeted or first full validation. One
documentation patch had a mismatched README context and made no writes; source
inspection and a corrected patch resolved it. Stronger lifecycle, interpreter,
binding/source and stream/callback work remain separately pending; no tests were
weakened or private protocol facts invented.
Catalog 118 records, 106 changed local Markdown links and whitespace PASS.

Full gate: **1786 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
C lifecycle change.

Validated SHA-256 snapshots:

| Artifact | SHA-256 |
| --- | --- |
| `target/debug/libios_ffi.so` | `b06afba0815f12c40a46b38edfa49aa0514b596dfc6810b4849751a6629a394d` |
| `target/debug/libios_ffi.a` | `e347e2dd86e9f1fbeb022864641b4f858292887df2817d52ffdaa98cd02ec7b1` |
| `crates/ios-ffi/include/ios_rs.h` | `03316632ab96011d668a6022cc8c5a98e6c4f2b1218e2870818f617b06b435ce` |
| `target/debug/ios` (0.1.13) | `24c44a2288cd7c2fa9ccd3d82a8426be576d81ee089ceb81010478a90d02043b` |

This host slice is accepted; A02/third class and the 31-task goal remain active.
No capability maturity upgrade, device operation, push, publication, history
rewrite or reference-checkout modification occurred. Next is the registered
streaming-install diagnostic callback: its existing core callback returns void,
and CLI startup/progress eprintln does not propagate writer failure. Remaining
JSON/callback/binding/source, L01/L03/A03 and platform/device conditions remain;
fourth class has not started.
