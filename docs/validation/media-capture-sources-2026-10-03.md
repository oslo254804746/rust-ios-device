# A02 media and packet capture native sources — 2026-10-03

Implementation `5dce4bfcce162b1ebe82b740631e1a0e893fb3d5`, eight source files,
starting clean HEAD `9a16f14` on `feat/device-service-updates-20260927`.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Seventeen previously erased native causes now survive: HID two Elapsed,
Display four AddrParseError/three Elapsed/two TryFromIntError, plist framing one
TryFromIntError, Bluetooth pcapng two TryFromIntError, and pcap one each Elapsed,
TryFromIntError and FromUtf8Error. The last pcap adapter formatted its original
error; it was outside the earlier anonymous-closure search. Original objects
remain available through Error::source and the existing fixed context.

HID close deadlines change from Protocol prose to existing Xpc(Io), classified
timeout. Display's native timers use existing Io instead of its unit Timeout;
that unit remains for the existing remaining-zero admission outcome. Display
local IP parsing changes protocol to invalid_input; received streamConfig integer
overflow remains protocol. Pcap IP-finder expiry changes protocol to timeout.
Local pcap/pcapng encoding length conversions change protocol to invalid_input;
the private plist-frame conversion changes its directly observed unknown to
invalid_input. Existing plist-frame outer adapters carry Io without flattening it.

Public DisplayError's eight, HidError's five, PcapError/BtPacketLoggerError's three
variants and private PlistFrameError's three variants stay. Public signatures,
root exports, fields, dependencies/versions/lockfile/features and classifier stay.
Affected returned variants, fixed phase/Display and correlations can change.
Migrate Protocol/Timeout/immediate get_ref matches to class/source traversal.
Private pcap FromUtf8Error retains original metadata bytes; explicit native
source/Display/Debug remains private data. Outer/context Debug and automatic
machine diagnostics stay safe.

Private length helpers expose the existing conversion for tests without giant
allocation. The Bluetooth block helper also uses checked_add(12), rejecting host
size overflow as source-free InvalidInput before conversion or writes. It does
not invent a native integer error. Admitted block lengths, record fields/order,
padding, timestamps, headers, writes/flush and unsupported-packet false stay.
Plist's configured maximum still rejects before conversion/writes; frame limits,
serialization, read policy and exact payload behavior stay.

HID state clears only after its existing send operation succeeds. **HID reports
do not wait for a reply acknowledgement.** Timeout keeps held keys/contact/local
closed state, and Drop still performs no async release. Display still marks its
control channel used before I/O, opens a fresh connection for later requests,
consumes the service and requests task abort before stop awaits, and preserves
its existing stop EOF compatibility fallback. A repeated stop after consumption
is still a local no-op, not proof the prior remote stop succeeded. Existing stage
budgets, RTP deadline/assembly, RTCP/drain behavior, caps, optional zero/missing
configuration, pcap counters/filtering and successful flows stay. This slice
introduces no new L01 cleanup, reuse, cancellation or device acknowledgement.

The production comparison passes after excluding native adapters/private helpers
and equivalent extraction of the three framing/pcapng length conversions. All
five HID, fifteen Display, three pcap, two plist and two Bluetooth map_err calls
remain. Bluetooth's new checked-add guard and service state ordering are reviewed
separately. Original shared test_util is byte-identical; lib.rs adds only a private
cfg(test) module, so root exports stay. Logs/snapshot:
`/tmp/ios-a02-media-native-code-audit.log`,
`/tmp/ios-a02-media-native-source-diff.patch` (including the new fixture file).

| Check | Result |
| --- | --- |
| All-feature Display / HID / pcap / Bluetooth / plist tests | 17 / 8 / 10 / 8 / 5 PASS; `/tmp/ios-a02-media-native-{display,hid,pcap,bt,plist}.log`, terminal 0 |
| Final native regression group / no-default library | 60 / 227 PASS; `/tmp/ios-a02-media-{native-final,minimal-final}.log`, terminal 0 |
| External public_contract all / no-default | 25 / 13 PASS; `/tmp/ios-a02-media-public-final.log`, `/tmp/ios-a02-media-native-public-minimal.log`, terminal 0 |
| Existing external Display exchanges | 3 PASS; `/tmp/ios-a02-media-native-display-public.log`, terminal 0 |
| Final core/CLI strict Clippy | PASS; `/tmp/ios-a02-media-clippy-final2.log`, terminal 0 |
| Full validation with workspace .venv Python | First full PASS; `/tmp/ios-a02-media-native-full.log`, terminal 0 |

Ten new internal tests and one external consumer establish actual local address,
device-configuration integer and packet-metadata UTF-8 faults, short Tokio expiry,
and original owned native bytes. The external public pcap reader rejects a real
framed malformed metadata response with its native source and exhaustive error
family; existing valid frame/record and public Display exchanges pass. Native
plist/pcapng integer and host-add overflow tests call private length helpers, not
multi-gigabyte allocations/writes. Existing pcap length rejection is strengthened
with category/source assertions rather than matching erased prose.

A private cfg(test) gated duplex initializes the actual XpcClient through H2/XPC
bootstrap. Writes then stall before any request bytes are sent. HID timeout keeps
state; reopening this controlled gate permits successful local sends and state
clearing, with repeated local close no-op. This is not partial-send reuse or device
delivery proof. Display exercises its actual control method with a short private
deadline, UDP receive timeout/zero budget and stop consumption/abort request.
It does not prove background-task joins, physical media negotiation, every stage,
allocator/architecture/platform fault or remote release. No real credentials,
device/daemon or Apple endpoint operation.

Before-fix logs show **50 PASS/10 FAIL**, terminal 101 in both commands:
`/tmp/ios-a02-media-native-before.log` (nine intended internal regressions) and
`/tmp/ios-a02-media-native-before-public.log` (one external regression). The three
length conversions were equivalently extracted before these fixtures ran, still
using the original lossy adapters; production equality verifies that extraction.
Post-fix native tests pass. First all-target Clippy found the new test helper had
been placed in a file also included by integration tests, whose crate roots lack
internal XPC names. It failed E0433/E0425/E0422, log
`/tmp/ios-a02-media-native-clippy.log`. The helper moved to independent cfg(test)
test_native, restoring the old shared file; final native/minimal/public tests and
strict Clippy pass. Production/gates/lints were not weakened; failure logs remain.

Full gate: **1932 Rust/13 Python PASS, zero FAIL, two existing ignored**. Fmt,
check, strict workspace Clippy, MSRV, 66 feature builds/four groups, 7+7 doctests/
three snippets, C ABI/owned malformed/checked list, Python 3.12 wheel/sdist
rebuild/install, 53 tooling tests, 118 help records, 38 raw cases, five host
workflows/22 device NOT_RUN, 15 CLI and three backup contracts pass. No separate
Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug SHA256
`7b8ce4f8c069a4eb7380c121c8c23b983c696db0fbe1a5a40bd75fa555e063c8`.
Source stayed frozen through full and afterward. About 18 GiB free at full start,
17 GiB after; no storage failure or cleanup.

Documentation closeout: 118 capability records, 109 local links across five
changed Markdown files (baseline 9a16f14), unchanged capability implementation/
maturity and historical evidence, and whitespace PASS. Logs:
`/tmp/ios-a02-media-native-docs-catalog.log`,
`/tmp/ios-a02-media-native-docs-links.log`. Source and full gate stay unchanged.

A02/class 3/overall goal remain active; class 4 has not started. The next read-only
scan finds 66 anonymous-adapter candidates in 28 service files, not 66 confirmed
native losses; formatted/.ok() and created/optional outcomes require separate
review. `/tmp/ios-a02-media-native-next-services.log` and
`/tmp/ios-a02-media-native-next-timeout-audit.log` record the candidates and callers.
WebInspector quiescence/startup compatibility consumes its own timeout as a named
outcome: retaining Elapsed must preserve that behavior rather than consuming every
IO TimedOut. Other services, DProxy textual observations, codecs/typed guards/
created outcomes, stream/JSON/callback/binding and L01/L03/A03/platform/device
conditions remain. All 31 tasks/subtasks and capability maturity stay. No push/
publication, history rewrite or reference checkout edits.

Follow-up: [discovery source evidence](discovery-native-sources-2026-10-03.md)
records `89bcd3f`, 22 WebInspector/Companion/Notification native adapters and
1945 Rust/13 Python full PASS. Service-issued budget getters preserve quiescence
without consuming generic IO timeout. This page's 1932-test media gate remains
historical; the overall scope and capability maturity stay unchanged.
