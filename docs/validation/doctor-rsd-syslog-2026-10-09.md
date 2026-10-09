# Existing RSD observations and resumable Syslog framing

Accepted on 2026-10-09, Linux x86_64. The full run began on 2026-10-08 and
finished across midnight. RSD implementation: `838c4ed6fc873b1a26533627fb32f563f3bc25e3`. Syslog:
`f07767406f8131250b21cb71f754db9b9ad0a66e`. All 486 product paths at the latter commit match the full-run manifest.
These are F01 and A03/L01 native host work items, not completed main tasks.

## Existing-route RSD

The tunnel feature adds ExistingRsdRoute, observe_existing_rsd and checked owned
observations. The caller supplies an already established IPv6 endpoint, optionally
an existing loopback userspace proxy. Direct routes preserve numeric scope/flow;
proxy routes reject scope/flow rather than silently discarding them. Ports must
be nonzero. A single checked total begins before native host identity and spans
dial, the exact 20-byte proxy prelude, H2/bootstrap, replies and all four fresh
compatibility attempts. Dropping the future releases only its own sockets; it
never stops or adopts the proxy/tunnel. Matching malformed Handshake metadata is
final, while missing fields remain unknown and do not trigger another bootstrap.
The legacy handshake wire and default parsing paths remain available.

Borrowed admission checks the complete directory before cloning retained values:
4096 UTF-8 bytes per field, 4096 services, 16384 feature values and 1 MiB retained
text. These are logical retained-metadata bounds, not raw decoded XPC allocation,
RSS or decoder CPU limits. UDID, directory, Port and Features omissions remain
Option; explicitly empty Features remains distinct. Canonical entries win over
shim entries even when incomplete. NotAdvertised requires a complete checked
whole directory and identity. Advertisement does not prove service execution.

Doctor adds --rsd-address, --rsd-proxy-port and --rsd-service. The route requires
an explicit --udid or IOS_UDID, and selectors validate before host/source IO.
Target identity must match exactly before any positive report attribution.
Only complete directory/port metadata permits the new positive CLI values;
missing metadata stays unknown, malformed metadata is protocol, mismatch is
precondition, and absence from a complete directory is service_missing. Selected
service observation opens no connection to that service. Schema 1 host-only and
schema 2 without RSD selectors retain their fields and five device-check order.
Optional RSD fields contain only transport categories, completeness and feature
state/counts; identifiers, endpoint, service names and raw Features are omitted.

No new Pair, tunnel activation, settings change, mounting or service execution
is involved. Existing-trust checks continue independently, so their earlier
failure can remain the overall machine-error primary even if RSD also fails.
Each individual report keeps its own typed class and original native source.
The cooperative total cannot preempt arbitrary synchronous OS work.

## Syslog framing

Cancelling next_message after consuming a prefix now retains that prefix for the
next call; successful NUL completion clears it. UTF-8 and vis decoding, empty
messages, wire and the existing 1 MiB raw-message cap remain. Infinite idle
waiting remains cancellable and has no fixed short request timer.

First EOF, native IO failure or oversized message makes the stream terminal and
retains its original error. is_usable reports local framing state. All subsequent
reads reject locally as InvalidData/protocol before cached tail or transport
polls. Discard that stream and use a fresh connection. This fixes the previous
oversize-tail continuation that could produce truncated apparent messages.
Cancellation alone is resumable; transport failure is not. No auto reconnect,
SDK binding or device stream support is established by this change.

## Validation

Eleven new Core RSD tests use actual H2/RemoteXPC duplex or native proxy TCP,
including fresh compatibility sockets, strict malformed/missing metadata,
retained limits, native error identity, partial replies, late poll results and
owned socket release. Sixteen new CLI RSD tests exercise the production Core
observer, target/schema/field categories, selector admission, cancellation,
listener preservation and strict peer EOF. Eight new Syslog test groups cover
consumed-prefix cancellation, UTF-8/vis, repeated/idle reads, exact/oversize caps,
terminal tail refusal, original native cause and fresh TCP release.

CLI affected tests: 1201 passed, zero failed, one existing ignored. Core affected
unit tests: 1557 passed, zero failed, one existing ignored; public contracts and
integration/doctests passed. Minimal tunnel,syslog: 509 passed, zero failed.
Five additional actual CLI raw captures check RSD selector errors, typed machine
categories, empty stdout, redaction and zero connections to an owned listening
USB/proxy trap. Final raw captures: 48 passed.

The final `CARGO_INCREMENTAL=0 python3 scripts/validate.py full --python
.venv/bin/python` exited zero in 768.209 seconds: Rust 2923 and Python host 13
passed, zero failed, two existing ignored. Format, workspace all-target check,
strict Clippy, MSRV 1.80.1, 66 individual feature checks/four groups, default/full
doctests (seven each), three downstream examples, linked C ABI, rebuilt wheel
and sdist with installed contracts, 73 tooling tests, 118-record capability
inventory, five host workflows, 95 CLI contracts and three backup contracts
passed. The additional 14 doctest executions are not counted again in the primary
Rust total. This aggregate success does not certify device workflows.

The initial CLI fixture needed an explicit SocketAddrV6 parse type. The initial
Syslog TCP fixture could cancel before receiving its sent prefix; unread RX can
cause Linux RST. It now waits for all prefix bytes to be read followed by Pending,
retaining strict EOF assertions. This changes synchronization without accepting
RST or weakening behavior assertions. Failed logs remain archived. The earlier
standalone RSD cancellation fixture likewise waits for its controlled partial
frame bytes to be consumed before asserting EOF.

Root integrated frozen proposals from independent worktrees and alone ran Rust
jobs serially. Product sources remained frozen during each run. Evidence:
target/validation/doctor-rsd-syslog-2026-10-08/, including
`full-rsd-syslog-result.json`, `full-rsd-syslog-freeze.json`,
`frozen-implementation.patch` (all new files included) and `accepted-evidence.json`.
Both the live paths/hashes and all committed product blobs were verified.

Full log SHA256: `67962745cb79e0e6e6d849d069182a9c46eedcb708b8ed4e6c5054e5c7d26c19`.
Frozen patch SHA256: `b42ba769d7bfad2e957ed4e1ee814c4483c7731e1e02108d15d8d355dfaa7545`.
CLI SHA256: `691b9022c5325d471c696275f395d1a82f1344056493ecf268009aa2667de315`.

## Remaining acceptance

F01 platform USB enumeration, remoted coexistence, pairing-directory diagnostics,
physical RSD routes, DDI usability and representative device workflows remain
open. Existing routes are caller-selected observations, not general route setup
or platform readiness. Twenty-two device rows remain NOT_RUN; selected authorized
device, iOS/USB/Wi-Fi states and workflow fixtures are missing, as are macOS and
Windows/UNC runtime evidence. Blocking filesystem work remains physically
uncancellable and runtime exit may wait for its worker. A03 Python/C handles,
context/async/GIL, cancellation/ownership, bounded queries and runnable workflows
still need implementation and binding validation. No main-task or maturity
upgrade follows. Q04 and all children remain excluded.
