# L02 known metadata ownership and observed route freshness

This continuation starts from delivery commit `e9cf144c64ccc37d43ad3630222081564a8dd088`
(product source `88d8f707f6f23bc72b9c0aa80230ad0739752f03`). The original
[review](C:/Users/wangbaofeng/.codex/visualizations/2026/10/10/01a124fe-3525-7463-bba4-2e4d2788e644/review/REVIEW.md)
and the original [L02 definition](../engineering-roadmap-2026-09-30.md#l02多设备隔离)
remain the acceptance scope. L02 is not declared fully complete by this document.

## Product changes

The existing 64 MiB publication admission charge now includes known TCP Lockdown
host capacity, the native handle object and its owned UDID/address capacities,
the manager table key and row, and the two distinct retained route strings.
String capacities are charged rather than their displayed or JSON lengths.
The public `ConnectedDevice::retained_metadata_bytes()` still explicitly excludes
opaque errors and runtime allocations.

One `Arc<OwnedSemaphorePermit>` is shared by the published row and the original
native retirement registry entry. Removing the row alone cannot release the
charge while a runtime awaits its original joins. Certifying joins alone cannot
release the charge while the row still holds `ConnectedDevice` metadata. A failed
or cancelled unpublished result keeps its admitted charge in the same original
registry until retirement completes. No new worker or detached task is created.

Each published row also retains the exact route actually used by Source, separate
from a manual rediscovery filter. A complete admitted paired discovery snapshot
retires an old Mobdev2 publication if its observed UDID/host pair has disappeared.
An address change therefore reconnects through Source and receives new connection
metadata. The cleanup compares the exact captured publication identity so an old
browse cannot remove a concurrent replacement. Failed or rejected paired
snapshots preserve the previous owner and their original source error.

## Current boundary audit

| Retained owner | Existing bound and ownership | Byte conclusion |
| --- | --- | --- |
| Published `ConnectedDevice` and manager row | Shared 64 MiB known-capacity admission, table and original retirement owner | Covers the explicitly charged owned buffers; not all heap memory |
| Lifecycle gates and manual desired states | One 4096-key namespace; selectors at most 4 KiB; old tickets keep their canonical gate | Count and selector-length bounded; outside the 64 MiB charge |
| Failed connection records, retry episodes and Source originals | One shared 4096-slot admission; original-cause receipt shares its existing permit; version/route/incarnation checks; exact owner rollback | First/latest original objects and counters are bounded per retained identity; their dynamic allocations remain outside a measured global byte bound |
| Scan candidate and discovery snapshots | Raw rows/fields/text and complete candidate admission; no partial rejected snapshot retained across connection waits | Logical length limits; Source allocation and capacity remain separate |
| Abandoned scan originals | Eight fixed scan slots, three independent acquired source slots per scan | Original `anyhow::Error` objects preserved; opaque payload bytes unknown |
| Native retirement originals | Fixed 4096 runtime slots; first native failure/expiry kept separately, certified original joins | Runtime and original dynamic error payload bytes unknown |

`dyn Error`, `std::io::Error` custom payloads and `anyhow::Error` expose no reliable
owned-heap accounting operation. `size_of_val`, Display length and JSON size do
not charge arbitrary buffers retained inside those originals. This continuation
does not discard, stringify, redact-replace or truncate an original cause to
claim a byte limit. Implementing a strict global original-error payload byte cap
requires sized error admission at the creation boundary (including third-party
and native sources), or a separate explicit ownership/storage design. The
current fixed-slot history is a count limit, not that byte cap.

Automatic Source failure retention has now been implemented separately from the
asynchronous failure-table writer. The Source Ready(Err) poll moves the original
`anyhow::Error` into a receipt before an outer timer, cancellation or caller drop
can discard the returned failure. The current same-identity/incarnation receipt
keeps the first and latest originals and a cumulative count, without accumulating
an unlimited chain of intermediate retry errors. It shares the already admitted
failure permit with the retry episode/failure row. A fresh observed identity,
recovery, stop, clear, or terminal close retires the corresponding stored receipt;
partial discovery preserves it. An error acquired in a clear/close poll is stored
as a retired historical receipt and cannot resurrect a current device row,
backoff or readiness.

`DeferredFailure` carries the same receipt to failure-table completion. Actual
successful insertion marks Recorded. A table-write failure returns the original
bookkeeping error with a safe `SourceFailuresContext` containing the independent
Source originals; native `Elapsed` stays downcastable. Caller drop while the
writer is queued marks Abandoned rather than inventing a timeout or a successful
write. A successfully recorded prefix retains its Recorded state if a later row
fails. Returned receipts are separate owners: an explicit manager clear releases
manager ownership, while a caller retaining a returned dual-cause report still
owns that report's original admission permit. Diagnostic Display/Debug prints
classes, counts and completion states, never Source strings or endpoints.

## Lifecycle and freshness rules

| Event | Actual production rule | Evidence or remaining limit |
| --- | --- | --- |
| Same-device competing start | Canonical device serial lock plus current ticket and four shared connect permits | Existing lifecycle/parallel gates |
| USB detach/replug | Complete USB UDID/native-ID observation prunes matching old publication; final publication checks current attachment | Existing identity/publication gates |
| Paired address change or disappearance | Complete admitted UDID/host view prunes exact old Mobdev2 publication | Added freshness gates |
| Incomplete discovery | Preserve previously observed owner and original discovery cause; do not certify detachment | Existing source gates and added failed-browse gate |
| Local native runtime stops | `is_alive()` fails, row removed, new Source attempt follows retained selected route | Existing native intent/reconnect gates |
| Ordinary per-device stop | Durable Stopped intent survives ordinary scans and replug | Existing real HTTP/native intent gates |
| Explicit global clear | New reusable global epoch resets desired states; old in-flight tickets cannot publish into it | Existing clear and old-ticket capacity gates; unlike ordinary device stop |
| Terminal server shutdown | Closed epoch rejects all new admissions; original native registry retains stop/join ownership | Existing shutdown/native retirement gates |
| Same-address remote service restart while local runtime remains alive | Live shortcut observes local liveness only; it does not refresh cached RSD directory merely because time passed | Remote freshness and physical restart remain unverified |

The new paired observation does not prove current remote reachability, fresh
same-address RSD service metadata, Wi-Fi discovery freshness, or any platform or
physical-device condition. No real device was accessed for this continuation.

## Required root gates

No Rust build, Cargo command, indirect Rust build, or runtime test was run by the
editing subagent. The root agent owns serial compilation, formatting and log
retention. At authoring time the seven metadata/freshness cases and five original
Source cause cases are **NOT_RUN**:

```text
cargo test -p ios-core --all-features retained_metadata_charge -- --nocapture
cargo test -p ios-cli --all-features freshness_ -- --nocapture
cargo test -p ios-cli --all-features native_metadata_charge_ -- --nocapture
cargo test -p ios-cli --all-features failure_cause_ -- --nocapture
```

The first gate exercises spare allocated TCP/native identity capacities. The
second traverses actual production manager scans for an alive address change,
complete detach, failed browse with original typed IO error, and older discovery
versus concurrent publication. The last traverses actual smoltcp reader/writer
retirement and requires the original shutdown certificate, peer EOF and task
baseline, with the table and join owners released in each possible order.

The Source-cause gate uses an actual refused localhost TCP endpoint and verifies
the exact original IO object addresses through the production automatic scan,
same-identity retry, native-ID change, complete absence, failure-slot exhaustion,
stop and clear. It also holds the actual failure-table writer after Source has
acquired its IO error, requiring the real timer and the independent original
Source receipt, and separately drops that queued writer's caller. A same-poll
clear fixture requires original-cause retention without current-row/backoff or
readiness resurrection. Discovery metadata is synthetic; the ConnectionRefused
is an actual Windows/host socket error. None of these cases substitutes for a
device or protocol acceptance result.

The full CLI/core gates and Clippy remain required after integration. These
fixtures do not certify process RSS or a global error-payload byte bound.
