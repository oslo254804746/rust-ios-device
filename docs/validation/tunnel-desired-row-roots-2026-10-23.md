# L02 escaped desired-listing Ticket/Snapshot roots · 2026-10-23

Current census found an actual ownership transfer: KnownVec.into_iter().next()
can move a DesiredRow out before the iterator and its backing/grant end. The
independent copied key fee is zero for an empty key, accepted by the existing
private selector contract. It cannot be used as a positive identity-holder fee.

Before the production change, one executed finite fixture used the SAME
coordinator and existing Lifecycle APIs for four rounds of stop/list/extract/
drop-iterator/clear/lazy-reset. Four escaped rows retained pairwise distinct
old global/device/desired versions, each key permit was 0, and the original A
debit stayed constant at each round. The rows were kept in a fixed stack array;
no strong marker observation vector or foreign bank supplied an A fee. Source
hash audit proves production and all old tests matched 697832c; only this cfg
test was added. Its source, frozen inventory, terminal PASS log and test binary
hash are archived under before/. This is a finite private-interface observation,
not an infinite production transcript, not an approved arena capacity, and not
a claim that 4096 gates or some chosen spare is sufficient.

DesiredRow now owns two independent actual roots: the existing lifecycle
TicketOwner and a private SnapshotOwner using the original complete
Strong<Snapshot, Origins2> allocation. Each root pays its entire G body and
authentic captured foreign-origin body grants before clone/allocation; duplicate
origins and G are deduplicated. SnapshotOrigins2 is exactly its epoch/version
field count, not a lifetime or arena ceiling. Root Clone shares the actual
allocation; only borrowing is exposed, with no Weak/raw/mutable/Pin API or new
unsafe. Snapshot desired/route/authority and original control identities remain
the original values. The original infallible Cell/Snapshot/Ticket getter and
control bodies are unchanged; this fixes the concrete returned listing owner.

Listing preserves original epoch/filter checks, row backing reservation and
copied-key admission/copy order. The prepared Ticket retains the original gate,
device/global fields, receiver and captured desired fence. Actual Ticket and
Snapshot root construction follows the key at the original row field positions.
Key N−1 still refuses before either new root. On a later root refusal every
partial key/row/root grant unwinds without modifying current intent or entering
Source. The original row/key pointers remain checked. Manual index iteration
only dereferences the same actual Ticket inside the root; index/route/canonical
borrows and all scan/Source/native/clock/error bodies remain unchanged.

The finite four-round test now funds exactly four pairs of actual root bodies,
in addition to the original lifecycle/gate and temporary row backing. Empty-key
admission remains supported, one coordinator is reused, old versions remain
distinct, and dropping the iterator refunds its physical backing while each
escaped row keeps its own positive root fees. Those fees return only when the
field roots end. No production pool or count limit was enlarged.

Two new Snapshot root tests execute G/epoch-A/version-C N−1 and full-but-closed
refusals with zero backing, partial rollback and unchanged identity counts; and
an exact complete allocation with Snapshot inside its real span, original
identity pointers, same-origin deduplication and same-root Clone. Closed-zero
last-root physical-free observation still sees all body fees held, then exact
refund; old identity Weak observers cannot upgrade afterward.

Existing listing tests preserve the original key/backing/pointer/refund and
empty/clear/terminal assertions. The exact success fixture now observes four
actual allocations in order (row backing, key, Ticket root, Snapshot root),
checks both root spans, and budgets only the measured new bodies. Its original
key transfers and last-String fee assertions remain. The two-row partial-key
fixture funds the first row's real roots to reach the same second-key refusal.
The old manual-index pointer assertion compares &*row.ticket, preserving its
same-actual-Ticket check after the field type changes.

Validation uses targeted tests plus one full CLI run, without repeating every
unchanged module filter. First strict compile caught the old wrapper-address
assertion; after terminal status it was corrected to the actual Ticket address.
An early fmt call also caught the declared test file before it was created;
fmt succeeded before the final freeze. Round2 commands all succeeded:

- snapshot-root: test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 1892 filtered out; finished in 0.00s
- desired-listing: test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1888 filtered out; finished in 0.00s
- scan-desired: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1894 filtered out; finished in 0.00s
- cli-full: test result: ok. 1881 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 244.58s

scan-desired selected zero tests; its command success is NOT functional test
coverage. The full CLI explicitly passed the actual manual index/pointer test
cmd::tunnel::tests::lifecycle_tests::global_metadata_tests::scan_desired_manual_index_retains_original_snapshot_refs_and_exact_backing_owner,
and 110 native-retirement cases (plus the original ignored baseline). Full CLI
also runs the original native owned joins/strict Ok(0) EOF, typed IO/deep pointers,
global/manual/admission, retry totals/max/backoff, callback and cancellation tests.
Remaining commands: strict CLI all-features/all-targets Clippy -D warnings,
build/fmt/diff, actual readonly manager1 (Source/usbmux trap0), host5 PASS/
0 FAIL/22 NOT_RUN, real_device_executed=false. All Cargo/build/fmt/test execution
was serial by the controller; tests used --locked and --test-threads=1.

725 frozen source hashes match. Six CLI-only changed sources (3 production/
3 tests, 1 new test file); Core/Cargo.lock unchanged from 697/ec25, old Core24
evidence reused and not claimed as rerun. Controller whole-body source guards
prove the exact original listing/index/control bodies and old fixture migrations;
no independent final agent implementation approval is claimed. Actual host and
verified-ios.exe archive SHA-256 c288e2370fdf34d7a735ec392e479286dd65fd92698b7b0c12b3660696b57e9b
Commands, logs, results, before/after frozen inventories, source guards/delta,
current and postcommit raw-HEAD proof:
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-desired-row-roots\round2

This covers escaped desired-listing field roots, not every raw Ticket/Snapshot/
GlobalEpoch getter or candidate/writer/manual/Watch/custom-Waker/Weak holder.
Physical marker arena and its complete control/retainer theorem, opaque/public/
Core/SDK/legacy/framework/task/waker/waiter/allocator/native/transport and all-mode
endpoint recovery/native exposure remain in scope. 128MiB is the admitted known
object pool, not heap/RSS. No Miri/other-platform/real-device result is claimed.
Main ledger remains 23 delivered / 6 missing development / L02 pending full audit,
effective30 with all Q04 descendants excluded; the original seven-item goal stays active.
