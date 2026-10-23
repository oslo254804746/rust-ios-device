# L02 scan candidate, writer and CURRENT manual Ticket roots · 2026-10-23

Candidate, FailureIntent and SuccessIntent now retain the existing actual
lifecycle TicketOwner. Manual scan indexes borrow the DesiredRow TicketOwner
itself, preserving the same root through USB and paired planning. Ordinary
USB/paired inputs keep their original input/node/batch preparation and permission
checks; the new Ticket root is prepared after coordinator admission and before
Candidate publication or Source. Resource refusal uses the existing Coordinator
error slot and preserves seen admission, so paired discovery cannot retry the
same rejected USB UID. It does not start a connection, receipt or backoff attempt.

Deferred failures carry the prepared root from the original Source boundary.
FailureIntent::new takes it into the original first Ticket field without quota
or allocation; the inner Option becomes None. This avoids a redundant strong
owner changing Ticket/gate/watch destruction order. Contested success outcomes
carry the same prepared root alongside the original version-only DeferredSuccess.
Ticket::pending_success remains infallible and keeps its original metadata shape.
Private direct-attempt fixtures prepare their root before calling Source; pure
writer fixtures prepare during setup. No late writer constructs a new root.

Manual success collection still reads CURRENT gates, epoch, pending tuple and
authority after Source. Preflight<'a> now binds the actual immutable Lifecycle,
whose authentic birth bank creates all fields in that late Ticket. Before the
FIRST scan Source poll it prepares a KnownVec backing and one complete actual
empty Ticket allocation for each original MAX_DEVICE_GATES completed-row slot.
This 4096 is the existing maximum collected-row bound, not a marker-arena capacity
or one-spare argument. Every empty allocation pays its whole G body plus its
authentic producer-A mirror if foreign; same G/A is deduplicated. bytes() includes
the actual vector backing and every actual root. Production pool/row limits stay.
The late collector can only use its bound Lifecycle, including after clear; it
places the exact CURRENT Ticket into that original storage without new quota or
allocation. Completed and SuccessIntent share that root, not an earlier snapshot.

The existing shared weak allocation now exposes private Prepared<T,F>: a real
capacity-one, length-zero Vec of the exact Allocation<T,F> plus its whole grant.
It allocates before T exists and places T with the original push/NonNull/forget
construction. The original strong/weak atomic and unsafe bodies are unchanged.
Strong::new preserves extra-before-value local binding, so allocation refusal
destroys the original value before refunding its extra-origin fee. Unused empty
storage is the first field and is freed before its grant; specialized prepared
origins follow that destination. No new unsafe construction algorithm is added.

Completed originally drops Ticket before its derived desired identity. The new
CapturedVersion stores that same captured identity before a Weak keeper of the
actual Ticket allocation. It is created from the root's actual intent_version,
clones the same marker/weak and exposes only borrowed Deref/Debug. Existing tested
Weak/downgrade/Clone/Drop bodies are enabled in production unchanged; upgrade and
mutation remain test-only. Last strong still destroys Ticket/gate/watch at its
original point. Remaining derived values keep only physical backing and G/A fees,
and their value drops before the final weak frees that backing and refunds fees.

Six new executed cases cover complete empty storage before an aligned !Unpin
value, same-address closed-zero placement and original physical-free fees;
allocation refusal value-before-extra destruction; destination/origin N−1,
closed and allocator refusal rollback plus G/A deduplication; post-clear CURRENT
placement with exact root span and original canonical/marker identities; derived
Weak retaining fees while global/device/gate payload is already gone, then value
Drop before physical free/refund; actual foreign Preflight maximum backings and
CURRENT writer closed-zero reuse; and actual scan foreign-origin N−1 refusal
with seen duplicate, zero connection/receipt/native/slot work and final refunds.
The CURRENT phase fixture directly exercises the private collector/writer; full
CLI retains the original late-new-gate/pending/authority/stop scan and native tests.

Old exact quota fixtures add only the measured new Ticket root body where needed:
the previous failure-global N−1 still reaches that same original global refusal,
and the existing success-key last-drop assertion includes the actual root refund.
Paired planning measures Option<TicketOwner> in its actual tuple backing in
place of the old Option<Ticket>. Original key pointers, errors, clocks, counters and pending rollback assertions
remain. Source guards compare complete original writer/Source/Ticket/control and
unsafe protocol bodies; native/retirement/Cell/source/allocator files are unchanged.

Round1 strict compilation caught one old pure outcome fixture using raw Ticket.
After terminal status it was migrated to setup ownership and exact root refund.
Round2 strict, weak5 and prepared2 passed; 13 original manual tests passed, while
the new CURRENT case's final refund was 32 bytes short because its own observed
FailureVersion clone remained live. After terminal status that test drops its
own observation and adds the allocation-refusal destruction-order case. Round3
strict and all eight targeted filters passed; full CLI had 1886 PASS/1 FAIL/13
original ignored. Its old paired-node fixture still measured Option<Ticket>,
giving an incorrect byte boundary. After terminal status only that sizeof type
was changed to the actual Option<TicketOwner>; source guards preserve every old
assertion. Round4's eight commands passed. The eight round3 targeted results are
reused with proof that production and all other source hashes are identical;
the final full CLI reruns those cases too. Serial controller validation:

- weak-allocation: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1894 filtered out; finished in 0.00s
- prepared-ticket: test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 1898 filtered out; finished in 0.00s
- manual-success: test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1886 filtered out; finished in 0.74s
- candidate-index: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1896 filtered out; finished in 0.00s
- manual-index: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1899 filtered out; finished in 0.01s
- candidate-root: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1899 filtered out; finished in 0.01s
- failure-global: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1899 filtered out; finished in 0.00s
- outcome-key: test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1896 filtered out; finished in 0.00s
- paired-node: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1899 filtered out; finished in 0.01s
- cli-full: test result: ok. 1887 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 244.76s

Strict CLI all-features/all-targets Clippy -D warnings, build/fmt/diff, actual
readonly manager1 (Source/usbmux trap0) and host5 PASS/0 FAIL/22 NOT_RUN passed;
real_device_executed=false. Tests use --locked and --test-threads=1. Full CLI
includes 110 actual native-retirement PASS cases plus its original ignored case,
owned joins/strict Ok(0) peer EOF, original typed/deep IO pointers, manual/retry/
deadline/cancellation and publication identity checks. No zero-selected filter
is counted. 726 frozen source hashes match; 21 CLI-only changes (7 production,
14 test, 1 new test file). Core/Cargo.lock unchanged from 06133/ec25; previous
Core24 evidence is reused, not rerun. Controller review/source guards and an
independent readonly actual-diff review found no outstanding actionable production
source finding. All 26 inspected source hashes match the final freeze; its report
is archived with the round4 evidence. The reviewer ran no gates or source edits
and does not claim final acceptance. Its earlier quota interruption and initial
design report are distinct historical evidence, not substitutes for this audit.

Actual host and verified-ios.exe SHA-256: 237bba0d797cc7e4b68a045d4ffa8d8133b9b326bd046e10154629cf2ff127e2
Commands, logs, terminal results, frozen inventory, source guards/delta, current
and postcommit raw-HEAD proof: C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-scan-ticket-roots\round4

Physical marker arena, all raw getters/snapshots/GlobalEpoch/standalone/Watch/
custom-Waker/Weak/public/Core/SDK/legacy/opaque retainers, synchronous producer
peaks and all-mode endpoint recovery/verified native exposure remain in scope.
128MiB is the admitted known-object pool, not heap/RSS. No SDK/CLI NativeSource
future is polled; no Miri/other-platform/real-device claim. Main ledger remains
23 delivered / 6 missing development / L02 pending full audit, effective30 with
all Q04 descendants excluded. The original seven-main-item goal remains active.
