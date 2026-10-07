# Tunnel shutdown state changes

Implementation: `7dc589bf4f26ead3597d991992f251d0ec29831e`, parent `1ff3db029375adbb99cf73087724ba8e789f34bb`.
Linux x86_64, 2026-10-07, Rust 1.97.1/MSRV 1.80.1, project Python 3.12.13.
The active goal remains all 30 tasks and subtasks excluding Q04: 9 complete,
21 unfinished (9 not started, 12 in progress or awaiting conditions).

Three races allowed a completed lifecycle change to be missed. The manager
checked a ticket before inserting a tunnel or updating a failure row, leaving
a gap in which close, clear or stop could finish. Scan admission separately
checked open before subscribing; a close that completed between those steps
could become an already-seen watch version, allowing USB discovery to run.

Private Ticket::commit now retains device-generation and shutdown watch read
guards through a synchronous mutation. A stale ticket skips the action with
the existing native OperationCancelled. Otherwise a lifecycle writer waits
for the action to finish. Publication and failure recording use this same
boundary while holding their destination table guards. Original early checks,
policy/liveness order, retry count/identity rules, slot ownership and record
formats remain. Failure recording quietly skips a cancelled commit as before.
The action must not await or reenter these watches; no guard crosses an await.

Scan admission keeps its original preclosed and checked-deadline order, then
rechecks monotonic terminal state after receiver/generation capture. Completed
close is rejected before source polling; ordinary clear remains reusable.
Test-only one-shot callbacks force the two capture/mutation boundaries without
holding callback mutexes or watch guards during callback invocation.

Six CLI files add 769/remove 26 lines, including 10 new tests and test-only
callback scaffolding. All preceding assertions and other product sources stay
unchanged. No public API, error variant, dependency, protocol, pairing, native
source, Core, binding or policy change. Manual/discovery/candidate budgets and
the existing fallback sequence remain.

The publication fixture pauses the actual device's second is_alive call, waits
for close to complete on another thread, then releases the request. It records
publication before clear so cleanup cannot hide a late insert. Request and
closing threads are recovered before assertions. Fixed cases cover actual fake
Usbmux, USB, Wi-Fi and existing-trust Mobdev2 fallback routes; only the fallback
case has two connect calls and one paired discovery. It also checks device
Drop, serial/permit release and terminal rejection after clear.

Other tests cover a commit accepted before close and later cleared; stale
stop/clear tickets skipping actions; and a watch writer waiting through a
guarded action. The latter uses a 100ms bounded observation plus explicit event
order, not a latency measurement. Scan tests force complete close before real
subscription, clear reuse with a once-only callback, closed/MAX versus active/
MAX priority, and close after capture without polling work. Failure tests force
close after the first check and verify no inserted row/full slot recovery;
stop/clear after that check leave an existing row unchanged, with fresh update
and later cleanup still working.

Negative control restored the original mutation call sites and original scan
capture logic while retaining test seams and the new helper needed by helper
tests. Session 9611 exited 101: 6 PASS/4 FAIL. It directly observed a returned
record after completed close, USB discovery once after completed close, a new
failure row after close, and changed existing metadata after stop. Publication
failed at its first Usbmux case and metadata at its first stop case; subsequent
routes/clear branches were not separately negative-tested. The fixture released
and recovered its publication threads before the expected old-body assertion.
The six passing cases include the retained new helper; they are not evidence
that an absent original helper was correct.

After restoring the fix, frozen pipeline 68894 exited 0; all nine gates ran
serially under the primary. Source hashes were checked between every gate and
at completion. Subagents drafted ignored proposals and reviewed; the primary
applied/completed the implementation and reviewed the merged changes. No
subagent ran Rust tools. Usage interruption during the final delegated review
did not stop the primary's verification or authorize a source edit during it.

| Accepted check | Result |
| --- | --- |
| New full/minimum behavior |10/10 PASS |
| Tunnel regression |162 PASS/1 existing browser ignored |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict CLI Clippy/format |PASS/PASS |
| Section11 complete Rust/Python split |2631 Rust/13 Python,0FAIL/2 existing ignored |
| Project .venv tooling |73 PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full CLI1025/Core1444/public52 PASS. Workspace fmt/check/strict Clippy,
MSRV1.80.1,66 features/four groups,7+7 doctests/three snippets,C ABI,Python3.12
wheel/sdist rebuild-install-import,tooling73/help118/raw38/host5/CLI95/backup3
PASS.22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. No maturity upgrade.

Patch SHA256: `5d2ebc562a6af6c82abf888ee21c3f35b70a943bc4320e11a0cd7b3133fbb91a`.
Actual debug ios SHA256: `31c07770a14b5ce988debc3e82048113500f47b459ffbdd8fdff62c2441de6c8`.
Committed patch matches the freeze. Ignored proposals, source copies, controls,
gate logs, exits/hashes and reviews live in
`target/validation/tunnel-shutdown-fences-2026-10-07/`; original proposals are
in `target/validation/tunnel-publication-commit-2026-10-06/`. Read-only reference
HEAD/status and excluded Q04 sources remain unchanged. No cache clean, push,
publication, history rewrite, reference edit or device/credential/Apple action.

A commit accepted before close may finish its response afterward; cleanup
still removes its owned record. close may synchronously wait for a guarded
action before the server's separate 2s cleanup timer starts. This does not
establish a complete hard shutdown deadline or preempt CPU within one poll.
Policy, physical liveness and attachment snapshots remain point-in-time checks.
Same-poll default fallback, other scan mutations, startup/whole-scan/metadata
budgets, native worker/server/blocking ownership and explicit joins, physical
freshness/reused IDs/restart/manual epochs, L03-L04 and classes5-9 continue.
Proxy clients already have an owned JoinSet and an existing abort/EOF fixture;
do not reimplement that ownership or equate it to whole-owner explicit joins.
No device ACK, RSS, long-run, performance or platform certification. Q04 and all
subtasks stay excluded, the UID finding remains unfixed and other original
acceptance requirements remain. The full goal stays active.
