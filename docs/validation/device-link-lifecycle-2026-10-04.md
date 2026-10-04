# DeviceLink lifecycle host acceptance (2026-10-04)

L01/class4 increment, tested implementation
`3db5c1835ad73bf881c1c6a334674bc566cfe641`, based on
`a3d9a7cbd5d4cd4a5b247861b724469da0f2102f`.
Linux x86_64; Rust 1.97.1/MSRV 1.80.1; Python 3.12.13.
Four source/test files, 808 insertions/24 deletions. No device acceptance or
capability maturity upgrade. See [roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [compatibility](../public-contract.md) and
[error contract](../error-contract.md).

DeviceLinkClient adds is_usable, set_operation_timeout and call_process_message.
One absolute configured budget covers a message, complete version exchange or
process send/reply. Default None preserves unbounded operations and idle reads;
separate send/receive calls remain separate owners. Zero/overflow reject before
I/O, actual expiry retains Elapsed in Io.get_ref. Synchronous encoding/decoding
cannot be preempted by this cooperative timer.

A private guard tracks bytes accepted by the stream, including buffered reads
and writes. Cancellation/failure after progress, transport errors/native EOF and
frame decoding failure invalidate; later operations reject before consuming late
or cached replies. Empty idle/write waits, unpolled futures and pre-I/O local
serialization/size errors can retain state. Complete process-envelope validation
stays outside the framing guard with its original reuse policy. Version parsing
is inside its complete handshake owner, so aborted negotiation after a consumed
message invalidates. No queue is added; opaque transport buffers stay owned until
Drop. Boxed helper futures remain directly owned, without a background task.

Disconnect now has an independent five-second write/flush cap, clamped by a
shorter configured budget. Duration::MAX is safely clamped for cleanup. Success
marks closed; further operations, including another disconnect, return existing
Protocol. Failed/abandoned close after bytes requires a fresh transport; empty
write expiry can preserve local state. There is no ACK, automatic disconnect,
asynchronous Drop, primary/cleanup aggregate, rollback or safe-retry guarantee.

The native EOF migration affects one old backup fixture: an invalid DeviceLink
now rejects disconnect before its gated writer is polled, retaining the original
Mobilebackup2Error::DeviceLink(DeviceLinkError::Io(UnexpectedEof)) directly. The
fixture checks that native cause, absent issued timeout, unusable state and zero
cleanup writes. It previously expected a cleanup timeout wrapping that cause.
Valid complete business/success/pending cleanup-timeout assertions remain.

Public stream_mut/into_inner signatures remain unchanged. Raw I/O bypasses
tracking and remains caller-owned; extracting or rewrapping the same stream does
not repair its protocol state. MobileBackup2 production, screenshot/raw helpers,
four-MiB plist codec/limits, message envelopes, original parsers/error mapping,
frontends and dependencies are unchanged. Backup raw transfer/session-wide
ownership and other cleanup retention remain pending.

| Owned host fixture | Evidence / boundary |
| --- | --- |
| Preflight/local input | Seven operation paths reject zero without I/O; ordinary Duration::MAX rejects. Unpolled futures, refused serialization and oversized outgoing plist retain state; next real send succeeds |
| Idle/fragmented/buffered receive | Empty caller cancellation and issued idle expiry preserve reuse; six-byte partial frame cancellation/expiry rejects complete late body. A warmed BufStream four-byte header alone commits a later stalled body read |
| Empty/partial send | A raw-prefilled 64-byte direction blocks the tracked send before any new byte and preserves state; a subsequently accepted exact 64-byte plist prefix makes expiry invalidate |
| Process call ownership | Caller cancellation after a complete request rejects a complete late process reply. Every later tracked entry rejects without another wire operation; Drop yields peer EOF |
| Shared call deadline | 4096-byte request blocks a 512-byte duplex send. Peer drains at 125 ms and replies completely after another 125 ms; the shared 200-ms budget expires. Restarted receive timing would allow that reply |
| Shared version deadline | Initial version arrives at 125 ms; exact VersionsOk is sent, ready arrives after another 125 ms. One 200-ms budget expires. Separate caller cancellation after VersionsOk also rejects late ready |
| Complete/error policy | Complete malformed process dictionaries keep framing reusable, followed by successful receive. Native EOF/BrokenPipe, malformed plist, oversized declared length and wrong handshake invalidate with original failure family |
| Disconnect | Exact wire and terminal local close; configured MAX clamp, short partial expiry/cancellation and an actual default five-second blocked cleanup expiry under a six-second fixture bound |
| External consumer | Getter/setter/raw accessor signatures, seven named Send futures and exhaustive three-variant error match compile; this fixture does not perform device I/O |

Twelve tests added: eleven internal/one external. Final all-feature full and
no-default-feature minimum pass twelve. All fourteen DeviceLink tests and
96 backup regressions pass, with one existing backup ignore. Five original CLI
16-KiB stack checks and strict Core/CLI Clippy/fmt pass. New fixtures use owned
pinned futures/joins; exchange/rejection waits have one-second bounds, with six
seconds for the actual five-second cleanup expiry. No new fixture tasks or
old-source negative control, benchmark or device/other-platform acceptance.

Preparation: first narrow session83458 terminal101 had test-only E0283 key
inference and E0716 temporary-lifetime errors; corrected narrow61628 terminal0
passed twelve. After strengthening fixture wait bounds, pipeline97516 reached
the old EOF cleanup assertion and stopped101: 95 backup PASS/one FAIL/one existing
ignore. The explicit native EOF migration above corrected its contract, without
relaxing other tests. Final pipeline97982 terminal0 passed minimum12,
DeviceLink14, backup96/one ignore, stack5, strict Clippy and fmt. Source audit
keeps all original DeviceLink/public-consumer bytes and all other old backup
bodies/inputs; version body matches after inner-call renaming and process
parser/error mapping remains exact. Reference HEAD/status stays read-only.

First full gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session24485 terminal0, validation/full PASS: 2204 Rust/13 Python, zero failures,
two existing ignored; CLI706/Core1343/public45. Workspace fmt/check/strict Clippy,
MSRV, 66 feature checks/four runtime groups, 7+7 doctests/three downstream snippets,
C ABI, 3.12 wheel/sdist/install, 53 tooling/118 actual-help/38 raw/five host/
15 CLI/three backup contracts pass. Chromium and installed 3.9/3.13 were not
rerun; 22 device rows remain NOT_RUN. No maturity upgrade.

Frozen source SHA256:
`7b7cfbce2b68fd3e1a08ad12c74f9d08bdef6eb62c6e1dffd2fa8abd07493bc2`.
Final ios debug binary SHA256:
`d2e81b9165a545eae25d97f8380719b948cc958eff43089d7f59806170398539`.
Source stayed frozen through terminal full/minimum results; no clean/storage
failure. About 7.2 GiB remained before full and 6.3 GiB during sdist build.
Logs/audits `/tmp/ios-l01-device-link-` are local; this topic retains outcomes.

Documentation audit PASS: eight documents, seven Markdown files/266 local links;
all 31 tasks/subtasks and previous evidence remain intact. The new roadmap entry
was moved to the end before audit, restoring the exact historical prefix apart
from its L01 status row. Tested commit, frozen source/binary, full/narrow/minimum/
regression/preparation provenance and read-only reference HEAD/status match.
Capability118 bodies/maturity/history remain unchanged with one new host entry;
the updated catalog also passes source and actual CLI-help validation.

L01/class4/all31 goal remains active. Next: MobileBackup2 Hello/command/host-work
gaps, raw transfers and complete operation cleanup. Raw borrowed sessions,
setup/service/multi-request totals, other service/long-stream/manual-pairing
ownership, blocking-worker/resource bounds, L02–L04/Q04 and platform/device
conditions remain. No device, credential, Apple-side operation, reference edit,
push, publication or history rewrite occurred.
