# A02 Python runtime initialization errors — 2026-10-02

Implementation `b064539f4e49830cad6b0c62a6efd9de21448529`, four related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python 3.9.25 / 3.12.13 / 3.13.15 installed bindings.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Python caches the original io::Result of Tokio runtime initialization. Ordinary
IO failure now produces safe RuntimeError with typed category, fixed
`initialize runtime` phase and unknown effects. It does not enter the former
expect/panic path, poison later ordinary attempts or invoke device work.
Lazy initialization and blocking work remain inside allow_threads; invalid mode
still rejects first. Existing signatures, successes and operation-error args
remain. The exceptional initialization path changes from a panic boundary to a
built-in exception, explicitly recorded for release/version migration.

| Check | Actual result |
| --- | --- |
| `python3 scripts/validate.py python-host --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | Final 13 PASS, zero FAIL/ignored; `/tmp/ios-a02-python-runtime-host-final.log`, terminal exit 0 |
| `PYO3_PYTHON=/home/oslo/projects/rust-ios-device/.venv/bin/python cargo clippy -p ios-py --all-targets --all-features --locked -- -D warnings` | PASS; `/tmp/ios-a02-python-runtime-clippy.log`, terminal exit 0 |
| `maturin develop --locked` via pinned uvx range | PASS with selected VIRTUAL_ENV/PYO3_PYTHON; `/tmp/ios-a02-python-runtime-develop.log`, terminal exit 0 |
| Selected venv `python -I scripts/python-smoke.py` after editable installation | PASS; `/tmp/ios-a02-python-runtime-develop-smoke.log`, terminal exit 0 |
| `python3 scripts/validate.py python-package --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | 3.12 wheel/sdist install and API/error smoke PASS; `/tmp/ios-a02-python-runtime-package.log`, terminal exit 0 |
| One fresh abi3 wheel installed on 3.9 / 3.12 / 3.13 | Three PASS; `/tmp/ios-a02-python-runtime-py39-accepted.log`, `-py312-accepted.log`, `-py313-accepted.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-a02-python-runtime-full.log`, terminal exit 0 |
| Review | Source/contracts/whitespace and 82 changed Markdown local links PASS before implementation commit |

The new host fixture evaluates one owned lazy failed initialization twice:
initialization count one, operation count zero. It retains the original typed IO
kind/data and verifies safe Python RuntimeError args, all four metadata fields,
no raw-secret Debug and no unsafe cause. A real current-thread runtime then
executes an owned ready future and preserves a native operation failure/category.
This does not induce actual Runtime::new thread/memory exhaustion or certify
cleanup after partially initialized host resources. Effects remain unknown.

Installed binding smoke checks the unchanged factory/method signatures and
actual safe failure metadata against its sole owned malformed ListDevices peer;
invalid mode makes zero connections to the owned endpoint trap. These normal
runtime-success/operation-failure flows do not replace the controlled runtime
failure fixture or validate a physical tunnel.

The first minimum-version artifact call failed because the package validation
had already cleaned its temporary run directory (`/tmp/ios-a02-python-runtime-py39.log`).
No wheel was installed and the subsequent 3.13 command did not execute. A new
owned directory `/tmp/ios-a02-python-runtime-wheel-p1qwwo9j` holds a freshly built
single cp39-abi3 wheel. Each interpreter installed that same wheel in a separate
temporary venv through:

```sh
python3 scripts/artifacts.py python \
  --directory /tmp/ios-a02-python-runtime-wheel-p1qwwo9j \
  --python /home/oslo/.local/bin/python3.9
```

The same command with selected 3.12/3.13 interpreters passed. Build log:
`/tmp/ios-a02-python-runtime-wheel-build.log`. Paths are host evidence, not
portable setup requirements. Two patch applications failed validation before
writing and were corrected after inspection; failure history is retained.

Full gate: **1747 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Format/check/strict Clippy/MSRV, 66 feature builds/four groups, 7+7 doctests/
three downstream snippets, C ABI/raw plus owned failure, Python 3.12 wheel/sdist,
49 tooling tests, 118 help, 38 raw cases, five host/22 real-device NOT_RUN,
15 CLI and three backup contracts pass. Existing PBKDF2 and separate-browser
ignored cases remain; preceding HTTP evidence retains actual Chromium acceptance.
Accepted ios 0.1.13 binary SHA-256 remains
`29cb95dfa5729d22c7480db4500f08261fb69ce8af3f5ea9a468e9e48d98d90d`.

This ordinary initialization boundary is accepted; **A02 remains in progress**.
Other runtime-internal panics, interpreter/allocation/argument/callback errors,
C filtering/void/port boundaries, later RSD probe/inner sources and stream/partial
semantics remain work. Unknown effects grant no retry/rollback/cleanup guarantee.
No device action, maturity upgrade, push, publication, history rewrite or
reference-checkout change occurred.
