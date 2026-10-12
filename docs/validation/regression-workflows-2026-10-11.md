# Q02 / F02 CLI workflow continuation — 2026-10-11

This work continues the independent review of `af20f6aa` and the repair package
at `88d8f707` / `e9cf144c`. These are host tools and synthetic protocol peers.
No real-device application installation, clipboard operation, image mounting,
pairing or XCTest execution was performed.

The admission/parser gates currently pass on Windows CPython 3.13. The first
actual application and XCTest gate encountered Windows exit `0xC00000FD`
(stack overflow) before its localhost peer observed a service operation.
That failure is retained as a product failure; argv/help parsing is not used
to replace the native workflow result. The parent repair task subsequently
reduced the future stack, preserved startup owners and DTX error replies,
and recorded the actual application/XCTest gates as passing at `c98c819153`.
The original stack-overflow observation is not erased by the later pass.

## Added observers

`regression_adapters.py` now drives the real Cryptex CLI sequence:
`ddi cryptex list`, `auto-install --restore-dir`, a second list, then exact
version uninstall and restoration of the original list. `approved-ddi`
specifies `path`, `identifier`, `version`, `build_id` and five `files` records
with root-relative `path`, positive `size` and SHA-256. The image is at most
2 GiB; metadata is at most 16 MiB. The manifest selects one DDI Cryptex identity
and four distinct components. Hashes are rechecked before an operation. The
manifest provides fixture identity; it never grants `install-scope`.

An existing DDI is refused without an install/uninstall. A partial installation
attempt is inspected in finally; only the exact expected version absent from
the initial snapshot may be removed. A different version is preserved and
reports failed cleanup. This is a snapshot-based ownership check; it does not
provide atomic exclusion against an unrelated concurrent installer. The
existing CLI still requests an online Apple TSS ticket. A localhost Cryptex
install peer has not been executed, and the scripted response tests below are
not native Cryptex protocol evidence.

The rich clipboard observer drives existing `pasteboard get/set/watch` and
`notify post` commands. `synthetic-clipboard` specifies the general pasteboard
plus `baseline` and `contents` representations (`uti`, `path`, `size`, SHA-256).
It accepts an approved single item with inline text/PNG and no metadata or
promises; content files are at most 8 KiB each. The scenario writes text, then
text plus PNG, compares full content hashes, posts two unchanged-counter
notifications, observes a 500 ms window without another output snapshot, and
restores the approved original representations. The negative window is an
output observation, not an invented notification delivery acknowledgement.

`regression_watch_capture.py` owns a POSIX process group, bounded readers and
in-memory records. It starts mutations only after the actual watch reports
`connected`, then sends SIGINT and requires `stopped` with
`native_reclaimed: true`, original reader joins and process-group absence.
Forced termination cannot substitute for this native receipt. Windows was
initially NOT_RUN because the runner lacked owned console-signal supervision.
The [2026-10-12 tool continuation](windows-watch-2026-10-12.md) implements that
owner and verifies actual CLI startup cancellation. A complete Windows device
clipboard workflow has not been run.
Empty/multiple-item/promised/metadata-rich baselines are refused before mutation
because the current single-item CLI cannot restore them faithfully. Sensitive
raw output remains in RAM and is not printed or persisted in the report.

## Actual application / XCTest peer

`regression_dtx_fixture.py` extends the original localhost usbmux/Lockdown TLS
peer. It decodes the streamed IPA local headers, payload bytes and CRC, observes
actual InstallationProxy requests, and speaks DTX process-control and
testmanager protocols. XCTest driver/bundle/runner readiness, selected
configuration, case events and result completion are all native messages.
The original peer workers must see client EOF and join; closing fixture sockets
locally cannot supply a passing EOF receipt.

The actual CLI gate checks application install/query/launch/stop/uninstall,
preexisting application preservation, partial-install cleanup, finite XCTest
selectors, JUnit/log outputs and failed-test exit. F02 gates additionally target
runner installation, lookup, channel/session/authorization/start/result faults,
stalled startup/result deadlines, log EOF, JUnit directory refusal, failure then
successful rerun, rerun startup/result failure and preexisting-runner ownership.
The history gate reads the production `.attempts.json` sidecar and verifies
that the initial failure survives a later passing attempt. The parent task
recorded all 12 actual application/XCTest methods passing after its repairs,
and 29 adapter tests executed PASS with one POSIX platform skip. The history
checks use the original `failure` field and actual first-failure text. An idle
peer observation window covers the unchanged CLI total deadline and cleanup;
the original EOF/RST and worker-join assertions remain strict.

## Observer continuation and remaining tool boundary

The two discovery observers now retain their production branch and safe
correlation receipts, with matching matrix adapters. Their implementation and
original runtime boundary are recorded in [the identity continuation](rsd-identity-receipts-2026-10-12.md);
InstallCoordination and Display/HID are recorded in
[the protocol continuation](protocol-observers-2026-10-12.md). The previous
“being implemented” statement described the earlier editing stage.

| Row | Current observable implementation | Remaining runtime boundary |
| --- | --- | --- |
| `private-mac-altirk` | `discover paired-identity` retains both exact mechanisms and advertisement type/index | Complete actual CLI/mDNS/current saved-record workflow and physical identity observations |
| `modern-rsd-wifi` | `rsd observe` retains direct/queued/legacy/passive bootstrap receipt | Actual queued/legacy/passive CLI and physical route workflows; direct CLI and Core wire paths have prior gates |

The first three existing host paths and all physical-device rows remain separate
from mock/scripted responses and localhost protocol evidence. Q02 remains
under full original-scope review: Windows owned console-signal supervision and
graceful observer cleanup are now implemented and separately verified; original
FUP deliverables and complete device watch evidence remain distinct. The WDA localhost CLI continuation is
recorded in [the WDA repair evidence](wda-continuation-2026-10-12.md).

## Commands executed so far

* CPython 3.13 `-m unittest discover -s scripts -p test_regression.py -v`:
  20 executed PASS, three platform skips.
* CPython 3.13 `-m unittest discover -s scripts -p test_regression_adapters.py
  -k FixtureAdmissionTests -v`: 16 PASS, including actual CLI help parsing,
  integrity/ownership gates, and explicitly scripted Cryptex orchestration.
* `-m py_compile` for the five regression modules: PASS.

The POSIX native watch gate and all hardware flows are NOT_RUN on this host.
