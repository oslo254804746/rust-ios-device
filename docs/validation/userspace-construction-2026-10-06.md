# Userspace tunnel native construction failure

Implementation: `19315169dda049d6ab1acd68414bf4976b147b47`, parent `838caab433615ea56a1cc39ca8cf72683bbcea21`.
Linux x86_64,2026-10-06,Rust 1.97.1/MSRV 1.80.1,project Python 3.12.13.
The active goal remains all30 tasks/subtasks excluding Q04:9 complete,
21 unfinished (9 not started,12 in progress or awaiting conditions).

UserspaceTunDevice::start_validated previously spawned packet reader,writer
and smoltcp tasks before binding its loopback listener and querying the port.
A native initialization error returned before the device owner existed.
Dropping the three JoinHandles detached the tasks; the owner's abort-on-Drop
could not run. Listener bind and local_addr now execute after the existing
IPv6/MTU checks and before channels,stream splitting or any task creation.
Every fallible constructor step precedes worker creation. The successful
four-task path,public API,dependencies,packet/proxy behavior and error types
remain. One Core file267+/3-,including one Linux-only regression; all old
assertions and all other product sources are unchanged.

The test uses the real public constructor and OS socket bind,not an error
factory. Its ordinary parent starts an owned exact-test child with a10s wait
and independent2s kill/reap budget. Only the current-thread child lowers its
soft RLIMIT_NOFILE to min(original,64),preserving the hard limit. Owned
/dev/null files fill the available descriptors until native EMFILE. Three
attempts preserve a real DuplexStream peer and check:

- Actual TunnelError::Io,aggregate Io classification and native source EMFILE.
- No additional alive tasks using Tokio1.50's stable current-thread metric.
- Owned stream Drop exactly once,zero reader/writer polls and real peer EOF.
- A yield does not expose delayed workers or retained streams.

The normal path drops owned filler files and checks limit restoration. A
subsequent real successful bind starts four tasks with a nonzero port and an
actually polled packet reader. Owner Drop releases the stream and returns the
task metric to baseline within the fixture's1s observation,with peer EOF.
This observes abort completion in this host fixture without proxy clients; it does
not add explicit joins,certify device cleanup ACK or measure L04 resources.
RAII restoration on assertion/unwind is source review,not a separate executed
failure-restoration test. Native local_addr failure was not injected; its
safety follows placement before every spawn. Numeric bind adds no DNS worker.

Old start_validated with the new test kept was executed as an isolated direct
exact child:session97374 exited101,0PASS/1FAIL. First attempt returned genuine
EMFILE,then printed alive3/baseline0/drops0/read0/write0 before its task-count
assertion failed. Later EOF assertions,remaining attempts and success control
were not reached. These are observed counts,not a claim that every old branch
was separately executed. Restoring the fix yields all three direct-child
attempts alive0/baseline0/drops1/read0/write0 and the successful Drop control.

Frozen pipeline62936 exited0. All11 gates ran serially under the primary:

| Accepted check | Result |
| --- | --- |
| Full-feature parent/direct exact child/minimum-feature test |1/1/1 PASS |
| Core tunnel regression |46 PASS |
| Minimum Core userspace and CLI all-targets |PASS/PASS |
| Original CLI stack/strict Core Clippy/format |5 PASS/PASS/PASS |
| Section11 complete Rust/Python split |2621 Rust/13 Python,0FAIL/2 existing ignored |
| Project .venv tooling |73 PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full CLI1015/Core1444/public52 PASS. Workspace fmt/check/strict Clippy,
MSRV1.80.1,66features/four groups,7+7doctests/three snippets,C ABI,Python3.12
wheel/sdist rebuild-install-import,tooling73/help118/raw38/host5/CLI95/backup3
PASS.22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. No maturity upgrade.

All tracked crate/script/Cargo hashes were checked between gates and after
completion. Subagents drafted an ignored proposal and reviewed; only the
primary applied product changes and ran Rust commands. Product sources stayed
frozen during every validation process. One initial audit assertion incorrectly
treated an await inside a spawned task body as constructor suspension; the
static audit was corrected without product changes or a test failure. Native
control/source-review metadata was finalized after the runner started; its
captured baseline/source hashes agree. Committed patch matches the freeze.

Patch SHA256:`981028a87d668173a4b132c8d8c5ee5b8310ac067cacb96a0caa0c88051da923`.
Actual debug ios SHA256:`0e9a4b375298a50bff627caa920cc931412426c299bed209ce45cf999115b6a7`.
Ignored proposal,source copies,control/gate logs,exits/hashes/reviews live in
`target/validation/userspace-construction-2026-10-06/`. Reference HEAD/status
and excluded Q04 sources remain unchanged. No cache clean,push,publication,
history rewrite,reference edit or real-device/credential/Apple operation.

Successful-device shutdown and proxy child ownership/joins,kernel workers,
blocking pairing,Axum delegated handlers/upgrades,shutdown/publication and
failure-metadata linearization,scanner terminal-capture races,startup/whole-scan/
metadata budgets,physical freshness/reused IDs/restart/manual epochs and other
L01/L02/L03–L04/classes5–9 remain. No socket/RSS/long-run/device/platform
certification. Q04/all subtasks stay excluded,the UID finding remains unfixed,
and other original acceptance conditions remain. The full goal stays active.
