# A02 on-demand RSD retry and lookup errors — 2026-10-02

Implementation `233348554e44022f6bca522d18728d45b62bab05`, eight related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

After the existing five attempts with 500 ms between attempts, completed probe
failure returns its final original CoreError/source instead of an opaque
Unsupported. Success stops retries; owner preflight failure returns immediately.
Skipped direct probing without local mdns remains a local Unsupported prerequisite.
Existing inner budgets and fresh-transport bootstrap modes are unchanged.
Neither the initial observation nor the legacy cached directory is updated.

Missing canonical/shim entries in a successful directory now use the existing
XpcError::ServiceNotFound with class ServiceMissing. This concerns that directory
snapshot, without certifying universal device support or current reachability.
MetadataUnavailable and NotAdvertised evidence remain distinct. The shared private
dial/prelude adapter preserves original IO and other CoreError sources; XPC service
reconnectors no longer flatten these to TLS text. Returned failure variants and
safe diagnostic correlation can change; enum definitions and signatures do not.
Screenshot fallback accepts its same specific service names through the typed miss,
and retains legacy cases. The migration is documented in the linked contracts.

| Check | Result |
| --- | --- |
| `cargo test -p ios-core --all-features --locked device::` | Final 70 PASS, zero FAIL/ignored; `/tmp/ios-a02-rsd-retry-host-fixed.log`, terminal exit 0 |
| `cargo test -p ios-core --locked --no-default-features --features tunnel device::` | Final committed code: 49 PASS, zero FAIL/ignored; `/tmp/ios-a02-rsd-retry-tunnel-only-accepted.log`, terminal exit 0 |
| `cargo test -p ios-cli --all-features --locked cmd::screenshot::` | Eight PASS; `/tmp/ios-a02-rsd-retry-screenshot-final.log`, terminal exit 0 |
| `cargo test -p ios-core --all-features --locked --test public_contract` | Twelve PASS; `/tmp/ios-a02-rsd-retry-public-contract.log`; repeated unchanged consumers pass in full |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-rsd-retry-clippy-final.log`; corrected source also passes full workspace Clippy |
| `python3 scripts/validate.py tooling-tests` | Corrected source: 52 PASS; `/tmp/ios-a02-rsd-retry-tooling-fixed.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | Final PASS; `/tmp/ios-a02-rsd-retry-full-fixed.log`, terminal exit 0 |
| Review | CoreError/XpcError/device public declaration files unchanged; source comparison is not a universal ABI proof. Diff/whitespace and 85 changed local links pass |

Owned ready probes count exactly five failures and preserve the final typed
TimedOut/source payload, stop on the second successful attempt, and return the
first owner preflight error immediately. A reserved non-listening TCP socket
produces real ConnectionRefused through the public service call after retries.
The independent initial Failed/Timeout snapshot and empty legacy cache remain.

Cached-directory tests resolve canonical/shim entries and reject misses through
both public raw/XPC service calls, with no listener connection. A new-directory
fixture routes a real public raw-service call through the existing 20-byte
userspace prelude, H2 preface/settings and XPC handshake to an owned loopback peer.
An empty successful directory yields ServiceMissing, then peer EOF with no service
dial. Frames/capture storage and a five-second total fixture budget are bounded.
The original initial failure/cache are retained, and dropping the local handle
closes its watch channel. No TUN, forwarding task, device service or Apple cleanup
acknowledgement is established by these host tests.

The adapter tests retain IO kinds/data and non-IO CoreError sources while Debug
hides private fixture values. Screenshot tests accept only the original named
missing services and reject the tested native RSD timeout/permission/refusal errors.
Full XPC reconnect-after-disconnect and physical screenshot behavior are not new
acceptance claims: this slice source-reviews the shared adapter wiring, rather
than claiming those stronger L03/device workflows.

The first full gate failed at one of 52 tooling tests: its static safe-Debug guard
rejected a test-only AttemptSource deriving Debug. Replacing that derive with a
fixed manual Debug kept the guard intact; corrected host/tool/full scopes pass.
The failed `/tmp/ios-a02-rsd-retry-full.log` remains failure history. One earlier
apply_patch context mismatch caused no writes and was corrected after inspection.

Final full gate: **1757 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict Clippy/MSRV, 66 feature builds/four groups, 7+7 doctests/three
downstream snippets, C ABI/owned malformed/checked listing, 3.12 wheel/sdist
rebuild/install, 52 tooling tests, 118 help records, 38 raw diagnostic cases,
five host workflows/22 real-device NOT_RUN, 15 CLI and three backup contracts pass.
Existing PBKDF2/separate-browser ignored cases remain; no separate Chromium or
installed 3.9/3.13 wheel rerun was required for this Rust owner change. The ios
0.1.13 SHA-256 is
`44d2553f82647b734705d76214a6031ea7246e2abd1eefd7aadcf7650ddd9db1`.

This host slice is accepted; A02/third class and the 31-task goal remain active.
Inner H2/RSD source flattening, remaining binding/void/port boundaries and
stderr/stream/partial semantics continue. The observed syslog stream failure
returning success and some application mutations printing human text under global
JSON require actual fixes and fixtures. L01 deadlines/cancellation, L03 reconnects,
platform and device acceptance remain separate. No capability maturity upgrade,
device action, push, publication, history rewrite or reference-checkout change.
