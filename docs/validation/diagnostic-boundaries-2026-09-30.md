# Automatic diagnostic boundary acceptance, 2026-09-30

Tested implementation: `395b90bd206f746ceb80af736402b316cc731a5f`.
S04 and Q02 remain in progress. See the [diagnostic contract](../diagnostic-contract.md)
for output purposes and experimental error-schema changes.

Linux x86_64, Rust 1.97.1 / MSRV 1.80.1 / CPython 3.12.13.
Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
PASS, `/tmp/ios-s04-boundaries-full.log`: format/check/strict Clippy, 1577 Rust
tests (zero failed, one existing PBKDF2 ignored), eight separate Python host
tests, MSRV, 66 feature builds/four test combinations, default/full doctests
(seven each), three downstream examples, linked C ABI, installed wheel and
rebuilt/installed sdist, capability inventory (118), tools and diagnostics.

The full gate ran 15 tool tests. One subsequent empty-output regression was
added and all 16 tool tests passed with `python3 scripts/validate.py tooling-tests`
(`/tmp/ios-s04-boundary-tooling-final.log`). No Rust code changed after the full
gate. `actionlint .github/workflows/ci.yml` and `git diff --check` passed.
From crates/ios-py, `uvx maturin develop --locked` also passed with VIRTUAL_ENV,
PYO3_PYTHON and PATH selecting `/home/oslo/projects/rust-ios-device/.venv`
(`/tmp/ios-s04-maturin-develop.log`); it installed only the local editable extension.

| Actual raw capture | Exit | Original secret hits | Masked-copy hits | Outcome |
| --- | --- | --- | --- | --- |
| CLI invalid port | 2 | 0 | 0 | PASS; parser kind/help hint present |
| CLI host validation | 1 | 0 | 0 | PASS; canonical forward phase present |
| CLI usbmux source error, verbosity 4 | 1 | 0 | 0 | PASS; list phase/category present |
| CLI root/Wi-Fi/pair/prepare/profiles/HTTP proxy/MDM help | 0 each | 0 each | 0 each | Seven PASS; normal usage present |
| C numeric failure smoke | 0 | 0 | 0 | PASS; synthetic UDID/invalid UTF-8 fail before device access |

`python3 scripts/diagnostics.py` produced the final CLI report at
`/tmp/ios-s04-process-final-capture.json`; all ten contracts passed, without
timeout, overflow or incomplete capture. The report contains only counts and
outcomes. Raw bytes remain in bounded RAM and are never printed or archived.
Tool positive controls deliberately put synthetic UTF-8/hex/base64/URL/JSON
secrets into original output: original detection fails the assessment even
when the masked copy has zero hits. Empty output cannot satisfy acceptance.

Rust tests compare genuinely secret-bearing raw source chains/protocol replies
to the production diagnostic views. Local HTTP fixtures capture actual manager
and WebDriver response bodies; a local WebSocket fixture captures the production
CDP failure transformation and checks id/sessionId/code preservation and error
data removal. Explicit Runtime values/exceptionDetails remain unchanged.
These are adapter tests, not complete F03/F04 workflows. Python tests inspect
actual PyErr str/Debug and repr, while connect_info retains the real selector.
Installed wheel/sdist smoke verifies the public ValueError boundary.

Development compilation failures (Parser import, anyhow trait coercion, test
router name and unavailable reqwest JSON helper) were fixed using existing
interfaces. Final checks do not hide those earlier outcomes. The change uses no
new dependency or device protocol assumption.

NOT_RUN: framework-generated extractor errors, complete output/derived-Debug
audit, live timeout/cleanup diagnostic capture, macOS/Windows/physical-device
acceptance. Q02 still needs its complete workflow harness, persistent safe
evidence and representative device matrix. S01–S03 remain separate required
controls. No pairing, DDI installation, TSS, erase/restore, push or publication
was performed.
