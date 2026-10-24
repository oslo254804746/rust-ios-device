# L02 attach the exact userspace handle before worker launch · 2026-10-24

The real userspace activation path now prepares its runtime, constructs the
original TunnelHandle and attaches that exact Arc to the reservation before
spawning or polling any worker. It then launches the same prepared reader,
writer, smoltcp and listener. An attachment refusal drops the original unpolled
futures; a stop requested inside attachment returns typed OperationCancelled
before entering the optional RSD probe. Public immediate userspace start uses
the same preparation followed by immediate launch.

The original authenticated stream is split once. Header/channel/listener setup
and all four actual outer gated worker futures are prepared before the first
spawn. PreparedWorker charges each complete concrete outer F. The four original
roles use prepaid stable OnceLock slots in a complete TaskHandles root; each
actual spawned JoinHandle enters its permanent slot before the next spawn, and
all four slots precede opening the gate. No temporary array of spawned tasks is
the sole owner of those joins.

A separate complete LaunchControl root contains phase/notification state and
the original Ledger reference, without task handles. Workers retain only this
control and their original runtime storage, preventing a root from owning its
own JoinHandle through its future. Metered roots reuse the existing sealed
allocator; only narrow visibility changes are needed, with no new unsafe code.
The no-bank path retains its explicit unmetered construction.

Named PreparedLaunch fields place all remaining unspawned futures before its
last LaunchGuard. Their actual Drop ends before the cohort is declared settled.
Unwind between installs or during a synchronous waker call retains installed
joins and issues abort requests through an allocation-free stack guard. During
an existing unwind, it avoids another arbitrary notification; a pending cleanup
wait can be cancelled and retried using the same retained actual slots. Abort
requests alone never certify completion.

Shutdown waits for the synchronous launch cohort to settle under its original
absolute clock, then collects all original slots and proxy children. The Ledger
also reports launch_pending separately from its actual pending native join
count. A dormant cohort can have zero joins while still being incomplete.
Snapshots read the launch fence before the join count, so observing its final
release includes all prior real task registrations. joins_complete requires a
settled launch and collected actual joins; no synthetic pending join is added.

Nine new tests cover real unpolled transport/future destruction at closed zero,
stop before launch, cancelled cleanup and retry, concurrent partial installation
and shutdown, panic after each actual slot installation, a synchronous custom
waker panic, complete original worker/proxy-child joins after an actual SYN, last
byte refusal and physical-free refund, and the real CDTunnel activation caller
with accept/refuse/stop attachment callbacks. They check original deep refusal
identity, no pre-attachment worker polls, exact returned Arc, held-runtime joins,
literal peer Ok(0), task baseline and complete final pool refunds.

Controller-only gates passed on the same frozen 736 sources:

- Strict Core and CLI all-features/all-targets Clippy, fmt and diff checks.
- New launch9, complete userspace75, activation10, native owner42 and manager
  shutdown7. Filters overlap. Each Core command also invokes ten integration
  targets with zero selected tests; those are not functional coverage.
- Userspace-only and Kernel-only library checks. Each retains 43 feature-specific
  dead-code warnings; these are compilation checks, not strict warning-free gates.
- Complete CLI1903 PASS / 0 FAIL / 13 original ignored, including native-retirement
  module110 PASS and one original ignored. No CLI fixture assertion changed.
- Current CLI build, actual readonly manager1 with usbmux trap0 and host5 PASS /
  0 FAIL / 22 NOT_RUN; real_device_executed=false.

Nineteen finite controller byte guards pass for original packet-link, kernel,
manager, manifests/lock, smoltcp/listener/client bodies, error recording, actual
join accounting and shutdown-clock implementation. Eleven Core sources change,
including three new files; CLI, Cargo manifests and Cargo.lock remain unchanged
from 3f5c9a4. Agent implementation was completed and reviewed by the controller
after agent quota ended. This is not an independent final review of L02.

Actual CLI and archived verified-ios.exe SHA-256:
`cd5c90cce7937e322245a712e5dbaf9cdfe9e7356294b4734b6e835ef96a14c5`

Commands, terminal logs, source/body fingerprints, archive and postcommit raw-HEAD
proof:
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-dormant-launch\round1`

The Kernel activation branch remains unchanged. This delivery does not provide
Kernel-private composition, private backend quiescence/port return, FIFO flush
barrier, checked prepare entrypoints, native exposure/readback/undo or exact
replacement/new-record publication. No production NativeSource/SDK constructor
was polled and no real device, interface, address or route operation ran.
128MiB remains admitted known-object capacity, not heap/RSS. Main ledger remains
23 delivered / 6 missing development / L02 pending full audit, effective30, with
Q04 and all descendants excluded. The original seven-main-item goal stays active.
