# A02 checked C device-list projection — 2026-10-02

Implementation `6abb92f15e2dcd1b4a879ed15f5987bf71fdd1e5`, eleven related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

The additive `ios_list_devices_checked` rejects embedded NULs in any received
UDID or connection type, returns IOS_ERR_CORE/Protocol, clears outputs to NULL/0,
and exposes no partial array. Valid and empty lists use the existing IosDevice
layout and ios_free_devices ownership. The old listing entry retains its
historical skip behavior. This checks C projection of received records;
upstream discovery completeness and current device reachability are not proved.

| Check | Result |
| --- | --- |
| `cargo test -p ios-ffi --all-features --locked` | 21 PASS, zero FAIL/ignored; `/tmp/ios-a02-c-checked-list-host-final.log`, terminal exit 0 |
| `cargo clippy -p ios-ffi --all-targets --all-features --locked -- -D warnings` | PASS; `-clippy-final.log`, terminal exit 0 |
| `python3 scripts/validate.py ffi` | Strict C11 header/link/example and 19/25/26-byte raw captures PASS; `-linked-final.log`, terminal exit 0 |
| `python3 scripts/validate.py tooling-tests` | 52 PASS, including six FFI validator tests; `-tooling-final.log`, terminal exit 0 |
| FFI tooling under Python 3.9.25 / 3.13.15 | Six tests PASS each; `-tools-py39.log` / `-tools-py313.log`, terminal exit 0; these are tool tests, not new installed-wheel results on these runtimes |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-a02-c-checked-list-full.log`, terminal exit 0 |
| Review | All fifteen header signatures from preceding HEAD retained, one added; status/layout declarations unchanged; whitespace and 86 changed local Markdown links PASS |

The linked executable makes five ListDevices requests over owned bounded
loopback peers returning binary plists: checked bad UDID, legacy mixed input,
checked bad connection type, checked valid input, and checked empty input.
The legacy result contains exactly the two valid rows in order. Both checked
failures have NULL/0 outputs and Protocol; successes clear the category to none
and free their arrays through the actual library. No other device request is
accepted. The Rust tests separately cover NULL output rejection before runtime
work, both NUL fields, empty lists, Unicode and retained original NulError.

Prepared CStrings keep Rust ownership until all checked records convert.
Failure drops those owned values; raw C ownership begins only after reserving
the output array. This is source-reviewed ownership plus executed error/free
behavior, not a heap profiler or sanitizer claim. The capture scans complete
synthetic secrets and the prefix before NUL, with zero original hits, bounded
output and completed owned cleanup. The tool rejects missing peer evidence,
wrong/empty output, nonzero status, unexpected stderr, leaks, timeout, overflow,
incomplete capture and failed cleanup; sanitized copies cannot certify raw safety.

Full gate: **1749 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict Clippy/MSRV, 66 feature builds/four groups, 7+7 doctests/three
downstream snippets, C ABI/owned malformed/checked calls, 3.12 wheel/sdist
rebuild/install, 52 tooling tests, 118 help records, 38 raw diagnostic cases,
five host workflows/22 real-device NOT_RUN, 15 CLI and three backup contracts
pass. Existing PBKDF2/separate-browser ignored cases remain; this C change did
not rerun the separate Chromium gate. The preceding Python runtime slice retains
its installed 3.9/3.13 wheel evidence. The ios 0.1.13 SHA-256 remains
`29cb95dfa5729d22c7480db4500f08261fb69ce8af3f5ea9a468e9e48d98d90d`.

Original work was recovered after the prior session exhausted its allowance;
its 19-test baseline and 21-test/linked fixture passes were checked, then the
final code and stronger validator were independently accepted above. No failure
is rewritten as passing evidence. This host slice is accepted; A02/third class
and the overall 31-task goal remain active. Void/port ambiguity, later RSD
retry/source preservation, remaining bindings and stderr/stream/partial contracts
continue. L01 deadlines/cancellation, platform and physical-device acceptance
remain separate. No device action, capability maturity upgrade, push, publication,
history rewrite or reference-checkout mutation occurred.
