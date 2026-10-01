# A02 CLI final machine failures — 2026-10-01

Implementation `70108b76ce12f5e8d726cb920c017f91c496c3c6`, nine explicitly related
code/tool/contract files. Branch `feat/device-service-updates-20260927`, Linux
x86_64, Rust 1.97.1/MSRV 1.80.1, binding runtime Python 3.12.13. No physical
device selected. [Error contract](../error-contract.md),
[compatibility](../public-contract.md), [roadmap](../engineering-roadmap-2026-09-30.md).

The additive global `--error-format=human|json` defaults to existing human
diagnostics. JSON opts into a compact newline-terminated final stderr failure
with schema version, typed class, canonical/fixed phase, safe diagnostic,
original exit, bounded typed confirmation observation and effect uncertainty.
Successful result stdout and existing exit 1/2 remain unchanged. Select the
format before invalid arguments; invalid/unrecognized selection falls back to
human. Parser recovery uses the same grammar with early help/version display
disabled only in recovery, without scanning positional values or error text.

| Check | Actual result |
| --- | --- |
| CLI `output::` filter | Final 13 PASS, zero FAIL/ignored; `/tmp/ios-a02-machine-output-bounded.log` |
| `error_format_recovery` filter | Final one PASS with input/missing command/invalid mode/literal `--`/nested help cases; `/tmp/ios-a02-machine-selection-fixed.log` |
| Strict CLI Clippy | PASS before final bound addition; final workspace strict Clippy covers the bound |
| Tool compatibility tests | Eight PASS (original three plus five new positive/negative cases); `/tmp/ios-a02-machine-validator-final.log` |
| Actual CLI compatibility fixtures | Final 14 PASS; `/tmp/ios-a02-machine-actual-final.log`, repeated in the final complete gate |
| Complete split gate | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` PASS; `/tmp/ios-a02-cli-machine-bounded-full.log`, terminal exit 0 |
| Diff/contract review | Primary parser remains authoritative; original human phase/text/exits and original eight output fixtures retained; nine files reviewed, 90 changed local links and whitespace PASS |

The actual CLI fixtures retain eight original JSON/human/empty/closed-pipe
cases, and add mode-selected successful list, malformed actual plist reply,
human/JSON invalid port, and closed help/version JSON failures. An owned server
accepts only ListDevices, never routes a device connection, and the argument
fixtures own the only usbmux endpoint and observe zero connections. Every raw
capture is bounded, cleanup completes, and no original secret hits occur.
The validators reject success/wrong embedded status, mixed output, wrong field
types/classes and a secret deliberately emitted in a structurally valid
envelope. These host fixtures are not physical device evidence.

The help-selection regression initially failed because DisplayHelp ended
recovery before matches were returned. Only recovery now disables those early
display paths. A first complete gate passed 1724 Rust before final review found
the inherited unlimited confirmation scan; that gate is not final-commit
evidence. Both human hint and machine guard lookup now inspect at most 32
sources. Cycle tests use a safety valve and count bounded source calls; 24/40
contexts test retained versus out-of-bound hints. The latter is Unknown with no
observed guard, not permission to perform an operation. Actual confirmation
checks are unchanged. The revised complete gate passed before this commit.

Final gate: **1726 Rust, eight Python PASS, zero FAIL, two existing ignored**.
Check/strict Clippy/MSRV, 66 feature builds/four groups, 7+7 doctests/three
snippets, linked C ABI/19-byte raw capture, 3.12 wheel/sdist rebuild/install,
46 tooling tests, 118 actual help records, 38 raw diagnostic cases, five host
regressions/22 device NOT_RUN, 14 CLI contracts and three backup contracts pass.
Accepted ios 0.1.13 binary SHA-256:
`09198cbdcac76a953bb4eb5517c9e532e5ab8ec4ecae447dd317239b77d44b33`.
Separate browser and Python 3.9/3.13 installed-wheel gates were not repeated.

This host slice is accepted; **A02 remains in progress**. The option does not
convert progress, tracing or stream stderr to JSON, and partial stdout may
precede failure. Ordinary stderr/callback/stream semantics, lost source types,
Ctrl+C classification, probe observations and HTTP/C/Python machine mappings
remain work. Unknown effects imply no rollback/retry/cleanup acknowledgement;
L01 retains orderly device lifecycle acceptance. No physical device, pairing,
DDI/TSS/erase/restore, maturity upgrade, push, publication, history rewrite or
reference-checkout mutation occurred.
