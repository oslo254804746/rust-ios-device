# A02 checked C cached-port queries — 2026-10-02

Implementation `32a7e0653e36c336d153caed9772fc2db1a486cc`, eight related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[FFI usage](../../crates/ios-ffi/README.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Two additive symbols, `ios_tunnel_rsd_port_checked` and
`ios_tunnel_userspace_port_checked`, return an int status and write a cached
unsigned 16-bit port to caller storage. A non-null output is cleared to zero
before checking the handle. NULL inputs return IOS_ERR_NULL/invalid_input;
a zero cached port returns IOS_ERR_STATE/precondition; a nonzero cached port
succeeds and clears the caller-thread category to none. They use the existing
status wrapper and category vocabulary, without a new status, enum or layout.

The original uint16_t queries retain their signatures, values, zero fallback
and non-updating category behavior. Their zero must not be classified from a
previous int call's category. Consumers requiring NULL/missing/success outcomes
can migrate to the additive symbols with a library providing them. The public
header and linked smoke carry both new signatures. The optional cbindgen export
configuration also includes them and the previously omitted category/checked
listing additions; the maintained public header was not regenerated or certified
by running cbindgen.

Queries read cached fields without runtime initialization, network work, resource
allocation or a new probe observation. Zero cannot establish kernel mode,
missing metadata, skipped/failed discovery, disconnect or explicit device
unsupported. Nonzero cannot establish that RSD or a proxy is currently reachable.
Neither result acknowledges device cleanup or supplies retry authority.
The handle must remain live, output storage must not overlap it, and concurrent
close/use or freed-pointer reuse remain unsupported. Void initialization/cleanup
reporting and stronger binding/lifecycle work are still pending.

| Check | Result |
| --- | --- |
| `cargo test -p ios-ffi --all-features --locked` | 23 PASS, zero FAIL/ignored; `/tmp/ios-a02-c-port-host.log`, terminal exit 0 |
| `cargo clippy -p ios-ffi --all-targets --all-features --locked -- -D warnings` | PASS; `/tmp/ios-a02-c-port-clippy.log`, terminal exit 0 |
| `python3 scripts/validate.py ffi` | Strict C header/link, listing example, ABI/raw capture, owned malformed and checked ListDevices PASS; `/tmp/ios-a02-c-port-abi.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-c-port-full.log`, terminal exit 0 |
| Source/header/config review | All 18 exported C function names appear in the header and cbindgen include list; original 14 signatures and two earlier additions remain. This name check is not generator/ABI acceptance; strict linked C checks the actual types/symbols on Linux |

Two new Rust tests call the actual exported queries. Owned metadata fixtures
cover both zero/nonzero and mixed cached ports, 1/u16::MAX, NULL handle/output,
output clearing, success/failure category replacement, legacy calls retaining
the earlier timeout category, saved static code lifetime, and a fresh caller
thread's category independent of its parent's permission failure. These fixtures
have no ConnectedDevice owner: they prove cached projection, not construction or
behavior of a physical tunnel. Strict linked C verifies both signatures and
NULL-output behavior; it does not supply successful physical handles. Existing
owned ListDevices tests and array freeing continue to pass.

No product/test failure occurred in the targeted or first full run. An initial
roadmap progress insertion landed before an earlier record; it was moved after
the actual preceding delivery before documentation acceptance. There was no
change to earlier accepted evidence. Whitespace and 104 changed local Markdown
links PASS; the catalog validates all 118 records. Evidence remains independent
from capability maturity.

Full gate: **1780 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
C query change.

Validated SHA-256 snapshots:

| Artifact | SHA-256 |
| --- | --- |
| `target/debug/libios_ffi.so` | `eb931a38ed84df9a1cefa4ba6d9a959ce208b5c4edc6b60b09691da586d352ae` |
| `target/debug/libios_ffi.a` | `3a270828a12284123b30af98208521855b62038447d3ff3b73c3716fe5af5be2` |
| `crates/ios-ffi/include/ios_rs.h` | `9986fa3100d23b6fe6390a84f2a0555c3b3fd5f28f38259e48f92ca04f96127f` |
| `target/debug/ios` (0.1.13) | `24c44a2288cd7c2fa9ccd3d82a8426be576d81ee089ceb81010478a90d02043b` |

This host slice is accepted; A02/third class and the 31-task goal remain active.
No capability maturity upgrade, device operation, push, publication, history
rewrite or reference-checkout modification occurred. Void initialization/cleanup,
other JSON/callback/binding/source and lifecycle boundaries are next. Fourth
class has not started; platform/device conditions are retained.
