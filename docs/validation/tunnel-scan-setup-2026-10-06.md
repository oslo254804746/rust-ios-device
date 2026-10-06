# L01/L02 tunnel scan setup waits

Implementation: `57d57999fddbe0234b502686b65b8500d7d50cfd`, parent `7ca396753cc89a38da980de9b0f558a7dd2eabdb`.
Linux x86_64, 2026-10-06, Rust 1.97.1/MSRV 1.80.1, project Python 3.12.13.
The active goal remains all30 roadmap tasks/subtasks excluding Q04:
9 complete,21 unfinished (9 not started,12 in progress or awaiting conditions).

The discovery deadline previously covered the scan queue and source futures,
but initial/post-USB tunnel table locks, USB coordinator admission and the
no-authorized-device failure/ready locks could wait indefinitely. Those waits
now use the same original checked15s ScanControl clock and generation. Clear
and terminal shutdown release their directly owned waits while the locks stay
held. The drive polls cancellation and native expiry before work, and checks
the nominal deadline on each work poll. A late ready lock cannot mutate the
table, clear failure rows, mark ready or insert a coordinator gate after expiry.

Successful USB snapshots retain epoch fencing. A native USB error already
returned by the source survives a same-poll epoch change. For ListError, table
cleanup observes the original clock/cancellation: cancellation skips stale
pruning and returns the primary IO error; cleanup expiry retains both errors
in private ScanCleanupFailure, whose source is the primary. Wrapper Debug and
the CLI safe human/JSON formatter contain classifications, not private inputs.
This is not a guarantee that arbitrary raw source Debug is redacted.

The setup clock ends at the paired reply. Paired candidate handoff, failure
reconciliation, final ready and each candidate's independent connection total
retain their existing behavior. No public API, dependency, native source,
Core, binding, policy or protocol change. Four CLI files726+/25-,16 new tests;
all preceding assertion functions and other product sources are unchanged.
The cfg(test) coordinator-count helper observes the actual private gate table.

The tests exercise actual TunnelAgentState locks and lifecycle gates:

- Initial and post-USB table locks held through product expiry; fresh scans succeed afterward.
- The same held table phases cancelled by reusable clear and terminal shutdown.
- USB coordinator expiry/cancellation before release, plus late release with zero inserted gates.
- No-authorized-device failure/ready locks held through expiry or cancellation.
- Late failure/ready unlock preserving rows/readiness; late post-USB unlock preserving the old publication.
- One cumulative clock across initial table, USB snapshot, post-snapshot table and coordinator wait.
- Native ListError with cleanup expiry or cancellation, retained primary/source and safe wrapper/CLI output.
- Native ConnectError/ListError triggering clear/shutdown within the same source poll.

The cumulative fixture uses1s total, two300ms held phases and650ms observation
after the second release. It detects a reset clock with scheduling margin;
it is not an L04 latency measurement. Combined helper tests cover their full
loops in the fixed run; a negative control stops at its first failed subpath.

With original refresh_once and ScanControl.drive restored but new scaffolding
kept, control session74607 exited101:0PASS/16FAIL. The control demonstrated
unbounded waits, late mutation/coordinator insertion and swallowed native
errors. It does not separately certify every branch of each combined helper.
The fix was restored, formatted and frozen before accepted gates.

Frozen pipeline91653 exited0; all nine gates ran serially under the primary.
Hashes of all tracked crate/script/Cargo files and the new test were checked
between gates and after completion. One subagent implemented the four scoped
files; another reviewed. Neither ran any Rust command. No source edit occurred
while a validation process ran. Follow-up agents only reviewed/drafted ignored
proposals, without applying them to product sources.

| Accepted check | Result |
| --- | --- |
| New full/minimum behavior |16/16 PASS |
| Tunnel regression |152 PASS/1 existing browser ignored |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict CLI Clippy/format |PASS/PASS |
| Section11 complete Rust/Python split |2620 Rust/13 Python,0FAIL/2 existing ignored |
| Project .venv tooling |73 PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full CLI1015/Core1443/public52 PASS; the Rust aggregate also includes workspace
integration/doc contract tests. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four groups,7+7 doctests/three downstream snippets,C ABI,Python3.12
wheel/sdist rebuild-install-import,tooling73/help118/raw38/host5/CLI95/backup3
PASS.22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. No maturity upgrade.

Frozen patch SHA256: `b6e8621bde201da0271ae9c0ef9117109eccaeb759599719f070de8f6c6301c7`.
Actual debug ios SHA256: `f89cd7fe0fecc52b448df11b619ebf7865c09d73c3ef75bf5cfba7d62bd3ecbb`.
Committed patch matches the tested freeze. Ignored source copies, control and
gate logs, actual exits/hashes and review are in
`target/validation/tunnel-scan-setup-2026-10-06/`. Reference HEAD/status remain
unchanged. No cache clean,push,publication,history rewrite,reference edit or
real-device/credential/Apple operation occurred.

This stops the next cooperative poll, not CPU within an already-running poll.
Shutdown/publication linearization, startup, whole-scan and post-paired handoff,
metadata/byte limits, native construction/worker/server joins, physical
freshness/reused IDs/daemon restart/manual epochs, other L01/L02, L03–L04 and
classes5–9 remain. Source future Drop does not certify a blocking worker join,
device ACK, resource baseline or performance. Timing changes need pre-1.0
release review. Q04/all subtasks stay excluded, the UID risk remains unfixed,
and other tasks retain their original acceptance conditions. Goal stays active.
