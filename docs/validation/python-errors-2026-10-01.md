# A02 Python error metadata — 2026-10-01

Implementation `465b3208f9fd65ee4a106d9785872315c50648ff`, six related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1. [Error contract](../error-contract.md),
[public compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

The native list/tunnel RuntimeError, invalid-mode ValueError and unavailable
proxy RuntimeError keep their existing safe single-string args and public
signatures. Four additive attributes supply schema version 1, typed class,
fixed phase and effect uncertainty. Native operation effects stay unknown;
mode/proxy rejection precedes that operation. Python causes do not expose native
device data. Metadata assignment failure returns its Python exception with the
safe original exception as its cause, rather than reporting success.

| Check | Actual result |
| --- | --- |
| `python3 scripts/validate.py python-host --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | 12 PASS (eight original plus four new), zero FAIL/ignored; `/tmp/ios-a02-python-errors-host-fixed.log`, terminal exit 0 |
| `validate.py python-package` with the same interpreter | abi3 wheel and sdist rebuild/install/smoke PASS; `/tmp/ios-a02-python-errors-package-fixed.log`, terminal exit 0 |
| `uvx --from 'maturin>=1.5,<2' maturin develop --locked` in `crates/ios-py`, selected venv/PYO3_PYTHON | PASS, then editable-installed smoke PASS; `/tmp/ios-a02-python-errors-develop.log` and `-develop-smoke.log` |
| `artifacts.py python --directory /tmp/ios-a02-python-errors-wheel-8dbd6wxf --python <runtime>` | Same new abi3 wheel installed in separate Python 3.9.25 and 3.13.15 venvs; both updated smoke PASS; `/tmp/ios-a02-python-errors-py39.log` and `-py313.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-a02-python-errors-full.log`, terminal exit 0 |
| Diff/links | Production boundary, safe causes, old args and visible signatures reviewed; whitespace and 77 changed local links PASS before implementation commit |

New host tests retain the explicit native IO source while checking safe Python
args, distinguish typed cancellation from EINTR/opaque prose, reject unavailable
proxy without retaining resources, and exercise a controlled metadata-assignment
failure. Installed smoke checks exact built-in exception types, fixed attributes,
safe args and the Rust-only GIL parameter's absence from the public method.
Invalid mode runs in a deadline-bound child against an owned endpoint trap with
zero accepted connections. A second child receives a malformed reply from its
sole owned ListDevices peer; requests are bounded and the peer cannot route to
a device. These are actual installed-binding failures, not physical tunnel tests.

The first Rust compile failed because abi3 PyString lacked the test's `to_str`
method; the test now uses the file's existing `to_string` conversion. The first
package smoke failed because the test incorrectly omitted the existing mux
wrapper from the expected diagnostic prefix. The corrected fixture checks the
old operation prefix and independent typed attributes, without changing the
production error or classifying natural-language text. Those failures remain
historical and do not substitute for the final passing logs.

Full gate: **1732 Rust / 12 Python PASS, zero FAIL, two existing ignored**.
The PBKDF2 and separate browser cases retain their existing ignored status.
Check/strict Clippy/MSRV, 66 feature builds/four groups, 7+7 doctests/three
downstream snippets, linked C ABI/19-byte raw capture, 3.12 wheel/sdist,
46 tooling tests, 118 help records, 38 raw cases, five host/22 real-device NOT_RUN,
15 actual CLI and three backup contracts pass. No separate S02 browser gate was
repeated. Accepted ios 0.1.13 binary SHA-256 remains
`7be336add56789ec4753be95de47d8f9964b9b9d59350fb310fef1ad23c9a053`.

This host slice is accepted; **A02 remains in progress**. Python argument
extraction/interpreter/allocation/callback and runtime-initialization boundaries
remain explicitly outside the mapped profile. HTTP/C mappings, probe ownership,
other lost sources and stderr/stream/partial semantics still need work; timely
physical cleanup belongs to L01. No device maturity upgrade, pairing, DDI/TSS,
erase/restore, push, publication, history rewrite or reference mutation occurred.
