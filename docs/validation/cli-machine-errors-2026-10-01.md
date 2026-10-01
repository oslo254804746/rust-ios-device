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

## Caller-observed cancellation

Implementation `ace87c1d9f2276a78eb7518c6a8d2cb3ff5b54e6`, eleven related files.
The additive private-fields OperationCancelled marker records local cancellation
and classifies through context or an explicit IO payload. Plain EINTR remains IO;
existing CoreError variants stay unchanged. WebInspector launch keeps its original
context/exit and caller-owned pinned future, but supplies the marker after a
successful Ctrl+C signal. Signal failure remains its original IO and outer timeout
retains Elapsed. JSON final class is consequently Cancelled, with effects unknown.

Final classifications passed twelve cases; initial cancellation filter passed
five unit plus one selected public integration. Minimal/default external consumer
passed nine before the Debug fix; final all-feature consumer passed twelve,
including fixed Debug and original exhaustive matches. CLI WebInspector passed
38 (original 34 plus four owned-pending/ready-signal/real timer/result cases).
Diagnostic safety tools passed seven and existing compatibility validators eight.
Initial consumer compile failure was corrected to test source absence without
trait-object equality. The first full gate failed the repository's safe-Debug
source check. The marker now renders only a fixed type name, and the unchanged
check plus the revised full gate passed. The failed gate is not commit evidence.

The actual CLI signal fixture owns the only usbmux endpoint, accepts one bounded
ListDevices request, withholds its reply, then sends SIGINT only to its owned
CLI child. It observes exit 1, a Cancelled/unknown-effects JSON error and peer
EOF; no pair record, device service or private protocol response is supplied.
Final actual CLI contracts passed **15**, preserving the preceding fourteen,
with bounded raw capture, complete owned cleanup and no original secret hits.
This validates the host signal/socket release path before device admission,
not physical cancellation or Apple teardown acknowledgement.

`python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` passed **1732 Rust, eight
Python, zero FAIL, two existing ignored**
(`/tmp/ios-a02-cancellation-final-full.log`, terminal exit 0). Other final gate
scopes are 66 features/four groups, 7+7 doctests/three snippets, check/strict
Clippy/MSRV, C ABI/19-byte capture, 3.12 wheel/sdist, 46 tooling, 118 help, 38 raw,
five host/22 device NOT_RUN, 15 CLI/three backup contracts. Accepted ios 0.1.13
SHA-256: `7be336add56789ec4753be95de47d8f9964b9b9d59350fb310fef1ad23c9a053`.
Original variants and caller ownership/selection/output diff were reviewed;
76 local links and whitespace passed before commit. This is not a general ABI
or device proof. HTTP/C/Python mappings, probe observations, other lost source
types, stderr/stream/partial semantics and L01 device cleanup remain pending.
