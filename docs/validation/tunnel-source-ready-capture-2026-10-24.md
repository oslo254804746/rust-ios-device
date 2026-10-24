# L02 Source Ready(Err) capture before original future Drop · 2026-10-24

An automatic Source can return a real connection error and change the lifecycle
epoch in its future's destructor. Previously connect_and_store_with_context
dropped that future before capturing the error. The actual admitted Source Drop
could therefore observe no durable receipt for the error it had just returned.

The existing input context and prepaid receipt capture now run in the actual
Ready(Err) poll, after the unchanged first-poll emit_start and before Source Drop.
Pending and successful results do not capture. Constructor admission failures
remain before attempt start, and the finished Source allocation and its fee still
end before unrelated device/output admission. No new quota is acquired here.

If Source Drop invalidates the ticket after capture, Ledger.finish retires only
the Active cause whose receipt pointer equals this attempt's original root. It
preserves the existing row version predicate and event/removal path. After the
Ledger guard ends, it completes that receipt as Abandoned; the existing
Pending-only completion rule preserves earlier Recorded/Failed outcomes. A
normal deferred failure remains Active/Pending until its original writer ends.
Clock, attempts, failures, backoff, limits and original typed errors are unchanged.

Two new tests wrap the existing localhost TCP ConnectionRefused Source with an
actual admitted concrete future. Its Ready poll holds all available pool permits
and closes the pool. Its own Drop runs before either original future allocation
or fee ends, observes the already captured receipt at zero quota, and optionally
clears the epoch using fully prepared original destinations. Tests require exact
first/latest IO pointers, one Source call, one acquired failure, exact Active or
Retired receipt identity, normal Recorded/fail_count1 versus stale Abandoned/no
failure row, and full final pool refund after the last receipt owner ends.

The new tests compiled against the original production function and both failed
at the receipt-at-Drop observation. The first repair also exposed a real Pending
completion gap after Drop invalidated the ticket; the exact retirement above
fixes it. These failed runs are retained as regression evidence, not passing gates.

Final controller-only round2 gates passed on the same frozen 733 sources:

- CLI all-features/all-targets strict Clippy and fmt/diff checks.
- New Source Drop2, complete failure-cause15, manual-admission12 and the actual
  Source constructor last-byte rejection1. These filters overlap.
- Complete CLI1903 PASS / 0 FAIL / 13 original ignored. Native-retirement module
  coverage within that run is110 PASS and one original ignored.
- Current CLI build, actual readonly manager1 with usbmux trap0, and host5 PASS /
  0 FAIL / 22 NOT_RUN; real_device_executed=false.

Only four CLI sources change, including one new test file. Core, manifests and
Cargo.lock remain unchanged from 7391635. No production NativeSource or SDK
constructor was polled; no real-device, native-interface, address or route
operation ran. The new tests themselves have no native tunnel workers/joins.
An independent read-only review checked all four source SHA-256 values and the
limited exact-body guards; it did not run tests or approve full L02.

Actual CLI and archived verified-ios.exe SHA-256:
`fbbbe11ea03611faae3ea39d7be3e1a24f4dfdfb7b6b84e7f31ca625c3d581d4`

Commands, terminal logs, baseline failures, final source fingerprints, read-only
review, archive and postcommit raw-HEAD proof:
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-source-ready-capture`
Round2 is the accepted result. Earlier incomplete/failed rounds are not acceptance.

This closes one error-capture boundary required by endpoint staging. Persistent
backend quiescence/port return, FIFO flush barrier, attach-before-launch, verified
native exposure/undo, exact replacement/new-record behavior and full L02 audit
remain. 128MiB is admitted known-object capacity, not heap/RSS. Main ledger stays
23 delivered / 6 missing development / L02 pending full audit, effective30; Q04
and all descendants remain excluded. The original seven-main-item goal is active.
