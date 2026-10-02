# A02 initial RSD probe ownership — 2026-10-02

Implementation `b2c002629b17fed302b087462eb0a955fdd23f4e`, twelve related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13. See the
[error contract](../error-contract.md), [compatibility](../public-contract.md)
and [roadmap](../engineering-roadmap-2026-09-30.md).

`ConnectedDevice::initial_rsd_probe()` borrows the owner's historical initial
observation without IO or retry. Private fields and new non-exhaustive enums
keep skipped, available and failed outcomes distinct. Skips record tunnel opt-out
or unavailable direct-probe build support; failures retain the CoreError reaching
the owner and its typed class. Safe Debug renders only fixed state/reason/class.
Original sources remain available to explicit Rust callers, subject to diagnostic
redaction when reported. Root and device module paths expose the additive types.

Legacy public fields, CoreError variants and optional `rsd()`/`into_rsd()` initial
directory remain. Later on-demand discovery neither updates this initial
snapshot nor the old cache. A successful directory can independently indicate
ServiceMissing, MetadataUnavailable or advertisement; a skipped/failed probe
does not prove a device-side unsupported reply. Private proxy dial failures now
retain native IO rather than flattening it to TLS text. No wire message, retry
count, timeout, implicit pairing or device setting changes.

| Check | Actual result |
| --- | --- |
| `cargo test -p ios-core --all-features --locked device:: -- --nocapture` | Final 63 PASS, zero FAIL/ignored; `/tmp/ios-a02-rsd-initial-host-accepted.log`, terminal exit 0 |
| `cargo test -p ios-core --no-default-features --features tunnel --locked device:: -- --nocapture` | Final 42 PASS, zero FAIL/ignored; `/tmp/ios-a02-rsd-initial-tunnel-only-final.log`, followed by full gate in the successful shell chain |
| `cargo test -p ios-core --no-default-features --locked device:: -- --nocapture` | 18 PASS, zero FAIL/ignored; `/tmp/ios-a02-rsd-initial-default.log`, terminal exit 0 |
| `cargo test -p ios-core --all-features --locked --test public_contract` | 12 PASS, zero FAIL/ignored; `/tmp/ios-a02-rsd-initial-public.log`, terminal exit 0 |
| `cargo clippy -p ios-core --all-targets --all-features --locked -- -D warnings` | Final PASS; `/tmp/ios-a02-rsd-initial-clippy-final.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-a02-rsd-initial-full.log`, terminal exit 0 |
| Review | Source/new owner file/contract diff, whitespace and 81 changed Markdown local links PASS before implementation commit |

Five new host tests exercise skip/failure/success and stable state codes;
original typed IO data and safe Debug; the actual skip-tunnel owner with unusable
synthetic credentials; a reserved non-listening loopback proxy port producing
real ConnectionRefused/IO; invalid endpoints without connecting; and actual
connection construction retaining errors/directory plus local handle release.
The existing owned proxy handshake fixture still verifies destination framing,
stable host identity and successful initial observation on two connections.
The external consumer checks borrowed root/module paths with a wildcard for
future states, retaining existing exhaustive CoreError matches and descriptor
literals. Empty feature metadata stays permissive.

The non-listening socket reserves its port throughout the dial: there is no
released-port race or device endpoint. Invalid proxy input never connects to
the owned listener. The same direct-input test, run without mdns, observes an
unattempted build-dependent probe rather than a fabricated failure. Connection
construction uses only a local watch channel, not a TUN or forwarding task;
channel release is not remote cleanup acknowledgement.

First device scope: 59 PASS with a dead-code warning for the internal skipped
variant in non-test all-feature builds. The variant is now compiled only where
constructible or tested, rather than suppressing lint. Final strict Clippy passes.
One patch application failed against a formatted context, made no partial edits,
and was corrected after inspection. Neither event is hidden as accepted evidence.

Full gate: **1747 Rust / 12 Python PASS, zero FAIL, two existing ignored**.
Format/check/strict Clippy/MSRV, 66 feature builds/four groups, 7+7 doctests/
three downstream snippets, C ABI/raw plus owned failure, Python 3.12 wheel/sdist,
49 tooling tests, 118 help, 38 raw cases, five host/22 real-device NOT_RUN,
15 CLI and three backup contracts pass. Existing PBKDF2 and separate-browser
ignored cases retain their explanations; the preceding HTTP slice holds the
actual Chromium evidence. No browser or extra interpreter rerun is asserted.
Accepted ios 0.1.13 binary SHA-256:
`29cb95dfa5729d22c7480db4500f08261fb69ce8af3f5ea9a468e9e48d98d90d`.

This initial-probe host slice is accepted; **A02 remains in progress**. Later
probe/retry observations and source retention, older inner H2/RSD source
flattening, Python runtime/interpreter/callback boundaries, C record filtering/
void/port boundaries and stderr/stream/partial semantics remain work. This
snapshot certifies no current reachability, safe retry, device capability or
Apple cleanup. Physical/platform acceptance remains separate. No device action,
push, publication, history rewrite or reference-checkout change occurred.
