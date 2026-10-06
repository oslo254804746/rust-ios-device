# L01/L02 manual tunnel connection deadline

Implementation: `d3ed1c33c14b2dcfdf83ef6c678cc1f451813176`, parent
`1782d1f4f487d3ba05c6d6ee0ddc3d8744bfd8f9`. Linux x86_64, 2026-10-06,
Rust 1.97.1/MSRV 1.80.1, project Python 3.12.13. The effective goal remains
all30 roadmap tasks/subtasks excluding Q04:9 complete,21 unfinished
(9 not started,12 in progress or awaiting acceptance/conditions).

Previously manual starts acquired the coordinator before starting their
15-second timer. Contention could therefore wait indefinitely, or consume a
separate uncounted period before connecting. The private ConnectionDeadline
now captures one checked absolute deadline before admission and reuses it for
serial/cache/permit/connect/trusted fallback/attachment query/publication.
Its biased native expiry timer and nominal-deadline work-poll guard preserve
Elapsed while preventing late ready work from advancing the operation. Zero
never polls source work; overflow is typed InvalidInput. Existing cancellation,
source IO, routing, fallback order and successful record format remain.

Four CLI files add427/remove41 lines, including13 new tests and a private
cfg(test) method that holds the real coordinator mutex. No dependency, public
API, native source, Core, binding or access-policy change. All preceding test
assertion functions and the rest of the manager body stay byte-identical.
Excluded Q04 sources remain at the withdrawn-experiment baseline.

The first eight tests were run against the original ensure_tunnel body with
the new test scaffolding: session81949 exited101,2PASS/6FAIL. Failures proved
unbounded coordinator wait, budget reset after admission, zero and overflow
admitting work, late coordinator release connecting, and late native success
publishing. The zero negative control reached only its first route before
failure; it is not a negative run of every transport. Cancellation and caller
Drop remained passing. After restoring the fix, session55001 exited0,8PASS.

Read-only review prompted five additional boundary tests and more scheduling
margin in the cumulative test. The final13 cases cover:

- Real coordinator lock held through product expiry, with later fresh success.
- Coordinator contention and pending connection consuming one total.
- Zero across default/usbmux/USB/Wi-Fi without source contact or publication.
- Unrepresentable duration rejected as InvalidInput before source contact.
- Coordinator released after expiry without connection work.
- Native future made ready after expiry without query or publication.
- Clear and terminal shutdown cancelling admission while its lock stays held.
- Caller Drop while queued, then a new successful admission.
- Four permits held through expiry, and separately released late, without source contact.
- Paired discovery and the actual mobdev2 fallback sharing one budget.
- A late paired reply unable to start fallback.
- An already-created device released when publication lock becomes ready late.
- Actual manager HTTP handler returning502/timeout without private inputs.

The two cumulative fixtures use a1s product budget,600ms held phase and650ms
post-release outer observation limit. These are causal budget-reset tests,
not L04 measurements. They still require the test runtime to be scheduled;
they do not establish cross-platform latency promises.

Frozen pipeline98985 exited0; all nine gates ran sequentially. Each gate
checked every tracked crate/script/Cargo file hash and the new test file;
the primary made no source edits until authoritative exit. Subagents only
read and reviewed, with no builds, tests, edits or commits.

| Accepted check | Result |
| --- | --- |
| New full/minimum behavior |13/13 PASS |
| Tunnel regression |136 PASS/1 existing browser ignored |
| Minimum CLI all-targets/original CLI stack |PASS/5 PASS |
| Strict CLI Clippy/format |PASS/PASS |
| Section11 complete Rust/Python split |2604 Rust/13 Python,0FAIL/2existing ignored |
| Project .venv tooling |73 PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full CLI999/Core1443/public52 PASS; Rust aggregate also includes the other
workspace integration/doc contract tests. Workspace fmt/check/strict Clippy,
MSRV1.80.1,66 features/four groups,7+7 doctests/three downstream snippets,
C ABI,3.12 wheel and sdist rebuild/install/import,tooling73/help118/raw38/
host5/CLI95/backup3 PASS.22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN.
No maturity upgrade or physical-device, Apple ACK, native join, resource or
performance certification.

Frozen patch SHA256:
`bfb31c4b522a1221b01a38efcd4679787fc6bc38e2c59c9693d5ab6cda21cae3`.
Actual debug ios SHA256:
`e7820bf0f7d375ef1edb283a5ca867d280eee475f8d5303a3d49b35b4548aef0`.
Committed patch exactly matches the tested freeze. Ignored source copies,
logs, exit codes, hashes, source review and parallel follow-up audit are in
`target/validation/tunnel-manual-deadline-2026-10-06/`. Read-only reference
HEAD/status remain unchanged. No cache clean,push,publication,history rewrite,
reference edit or real-device/credential/Apple operation occurred.

The guard stops the next work poll at the nominal deadline. It does not
preempt CPU inside an already-running poll or make insertion atomic with
concurrent terminal shutdown. Startup, scan internal lock waits, whole-scan/
metadata/handoff totals, physical freshness/reused IDs/restart/manual epochs,
Core construction/worker/server joins, other L01/L02, L03–L04 and classes5–9
remain. Direct `ios tunnel start` and scanned candidate timing were not
modified. Admission/late-result timing needs pre-1.0 release review. Q04 and
all its subtasks remain excluded, its UID risk remains unfixed, and other
tasks retain their original acceptance conditions.
