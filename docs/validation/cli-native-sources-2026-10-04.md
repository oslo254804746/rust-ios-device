# CLI native sources and cancellation: host validation

A02/class 3 accepted 2026-10-04, source
`03c71b53cfd653636733662cbf3a241a4c7ed10f`, baseline
`1f0b67fba777e53fcee2fe44fb3b5b2e869ee2be`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Twenty-seven CLI files, 567 insertions/137 deletions.
[Public contract](../public-contract.md), [error contract](../error-contract.md).

| Original objects retained | Count / owning role |
| --- | --- |
| Services, native XCTest plan and system clock | 81; original typed source/class, including opaque Unknown |
| Actual timeout Elapsed | 13; Timeout, original configured deadlines stay |
| Latitude/longitude, IPv6, Wi-Fi profile, bool and float Duration | 6; local InvalidInput |
| Received focus hex and DDI hex/decimal identifiers | 3; InvalidData/Protocol |
| GPX/P12 file IO | 2; original IO object and native category |

These 105 objects previously became formatted text or were discarded. Native
services now enter anyhow through Error::new, with the prior fixed context
prefix where appropriate. Error classes follow original types; SystemTimeError
and other unclassified causes can still be Unknown. Context Display can show
only its prefix while the native object remains in the chain. Input/received
IO kinds record boundary provenance, not a real OS error or a text classifier.
Private context Debug hides native/opaque text. Explicit source/Display access
remains private data under the existing redaction contract.

Four legacy parser Strings, for configuration enums and trace levels, are owned
opaque data with no native source. Their discarded upstream native causes
cannot be reconstructed. Four URL scheme/base unit adapters use source-free
input decisions. Five direct DDI received-shape/unsigned/missing and web timeout
guards gain source-free received/input classification. URL failures are only
source-reviewed here; no production fault or native cause is fabricated.

Accessibility inspection, display capture and HID use existing public
OperationCancelled. Successful signal completion now returns typed Cancelled;
HID previously ignored the signal result and could mislabel registration IO
failure as cancellation. It now propagates that original IO. A private signal
parameter enables owned future tests; production still supplies ctrl_c.
Deadlines, select order, operation futures and existing cleanup policies stay.
The focus hex decoder is privately extracted from the actual received event
path. It has Protocol provenance, distinct from local parser input.

Successful wire requests, selectors, serialization/formatting, limits,
configured durations and stream/cleanup policies remain. This gate does not
add total/idle/cleanup budgets, remote stop acknowledgements, connection reuse,
retry or rollback guarantees. Some input parsing still follows device setup;
the new class is not general preflight or evidence of no effects. GPX/P12 and
coordinate contexts omit raw paths/values. Returned classification, Display,
immediate source and diagnostic correlation migrate; machine v1 fields and
exit policies remain. Source-free outcomes do not gain artificial OS causes.

Eleven tests were added. Five compiled against original adapters and failed
(0 PASS/5 FAIL), then passed after source retention. They cover actual local
coordinate and received integer parsing, bool, HID/direct-XCTest timer expiry,
and the location file boundary. The location test also checks actual empty-path
OS NotFound before connection; its first old assertion failed earlier at input
classification, so this file subcase was not reached in that negative control.

Six further tests cover original typed XCTest plan errors, testmanager-stage
expiry, finite Duration overflow versus source-free guards and the 100ms floor,
received focus hex decoding, native/opaque context privacy and signal outcomes.
Actual pending/zero-budget Tokio timers retain Elapsed. Signal fixture errors
inject an original empty-path file-open OS failure through an owned signal
future; this is not a real signal-registration syscall failure or Ctrl+C run.
Its successful signal fixture proves typed cancellation without sending a
signal to the parent. The MCInstall local profile error is a host builder
fixture; these tests do not certify device configuration or DDI/TSS operations.

An initial typed-plan fixture hit the CLI's missing UI bundle guard (9 PASS,
1 FAIL); adding the unit-test flag reached the intended builder error. Strict
Clippy caught one needless return, corrected before final checks. Source review
corrected received focus provenance before full. These are preparation/review
findings, not accepted platform/device failures. Only final source is accepted.

| Check | Actual result |
| --- | --- |
| Final new tests | 11 PASS |
| Earlier full CLI / final canonical CLI | 677 / 678 PASS, one existing ignored each |
| Final fmt/strict CLI Clippy | terminal 0 |
| First canonical full | terminal 0; 2093 Rust/13 Python, zero FAIL/two existing ignored |
| Additional real binary machine cases | 6 PASS; exit 1, expected class/phase, no trap connection/stdout/raw data |

Actual binary cases cover latitude, longitude, missing GPX file, native Duration
overflow, non-finite timeout guard and invalid XCTest plan. Each owns a loopback
usbmux trap and bounded/reaped three-second child, passes an explicit synthetic
UDID and uses existing safe capture/assessment. No trap connection occurred;
no actual device/daemon/credential/Apple action. Effects remain unknown in the
general schema, even for these no-connect fixtures. Six cases do not certify
all 105 adapters or all command failure stages.

The final code audit checks all 113 changed callbacks exactly: 105 native,
four opaque strings and four unit decisions. It compares 27 complete production
token bodies after only the reviewed callback, five decision, cancellation,
two helper extractions, unused import and one rustfmt comma normalization.
The private boundary implementation is reviewed separately. Audit preparation
handled closure blocks, continued strings, char literals and optional commas;
those temp-script corrections did not edit frozen product source. Core/root/
classifier/public consumer, C/Python and dependencies are byte-identical.

Full: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. Fourteen Rust/one Python
groups, 66 feature/four groups, 7+7 doctests/three snippets, fmt/check/strict
workspace Clippy/MSRV, C ABI/malformed/checked list, 3.12 wheel/sdist,
53 tooling/118 help/38 raw/five host/22 device NOT_RUN/15 CLI/three backup
contracts pass. No separate Chromium or installed Python 3.9/3.13 rerun.
Logs use `/tmp/ios-a02-cli-native-`: before.log, new-restored.log, cli.log,
clippy-restored.log, narrow-final.log, code-audit-final.log, full.log and
binary.log. All source staged/frozen through full and documentation.

Source diff SHA256:
`8fe550bf0579e62bd038ca0438f23a2b44d229fdfe710fdb431a926ad3c0a4cb`.
Tested ios 0.1.13 debug SHA256:
`863bce33663dde0d69643f1bc08bf4090a03caee72e0f8f4f8e7582f3212e2c0`.
About 24 GiB free before full and 21 GiB after; no cleanup, storage failure or
duplicate full. No maturity upgrade, reference edit, push/publication or
history rewrite.

Eight documentation files are reviewed separately; seven Markdown files/181
local links, whitespace and exact source/binary checks pass. All 118 capability
records, implementation/maturity and historical evidence remain unchanged;
one accepted host entry is added.

A02/class 3/overall goal stay active; class 4 has not started. All 31 tasks
and subtasks stay. Next: finite acceptance of remaining source-free decisions,
stream/JSON and binding projections; lifecycle/platform/device requirements
remain with L01/L03/A03 and their owners. The
[preceding diagnostic gate](cli-diagnostic-sources-2026-10-04.md) is retained.
