# L01/L02 tunnel admission after shutdown

Tested implementation: `0d025dd3404082fb76adab20758b2e278b2d9464`, parent
`25d3667ab41ac3fe30a5028128dd58269eeecad8`. Linux x86_64,2026-10-06;
Rust1.97.1/MSRV1.80.1, project Python3.12.13. The active scope remains30 tasks
and all subtasks excluding Q04; L01/L02/class4 and the overall goal remain active.

Previously shutdown notification left the coordinator open, and table clear
created another usable epoch. An Axum request already waiting for its JSON body
could finish after clear and create another tunnel during graceful drain.
Private lifecycle shutdown now uses a terminal state that clear cannot reopen.
Notification closes it before async cleanup. Existing tickets are cancelled;
fresh manual routes and scans reject with the existing OperationCancelled.
Coordinator tickets also capture their original clear epoch before locking.
Ordinary clear still permits a fresh generation; restart requires a new state.

Four CLI files add334/remove8 lines. The main manager only adds the synchronous
close call before its retained shutdown bool. Original assertions, server owner,
native source/options, SDK/Core/bindings/dependencies, permissions/routes and
wire protocols remain. The excluded archive/classification sources still match
the restored baseline. No Q04 experiment or high-cost execution was resumed.

Nine new host tests cover:

- All manual route choices and fresh scan remaining closed after repeated clear.
- Pending connection cancellation while the tunnel table is held by another owner.
- Completed device release while its attachment query is blocked.
- Same-device serial queue cancellation before native source contact.
- Ordinary clear followed by a successful fresh manual start.
- Actual HTTP start body admitted before shutdown, completed after table clear:
  production handler returns502/cancelled, no source contact or late publication,
  graceful drain completes and listener closes.
- Pending discovery cancellation, probe release and no ready publication.
- Server caller Drop leaving its retained state clone closed.
- Coordinator lock wait across clear/terminal close rejecting the old epoch;
  this test releases the lock before observing the result.

These use private owned source/device fixtures and actual router/listener paths.
There is no public fake backend, physical device or Apple operation. Request
entry and table clear are observed before releasing the body; timing is not a
delay assumption. Local Drop/EOF/permit checks are not native worker joins.

The compiled old production control exited101:1PASS/5FAIL. After the terminal
change the original six tests exited0:6PASS; three further boundary tests were
then added. The final frozen pipeline session11055 exited0 with all eight gates
passing; source stayed frozen until its authoritative exit. One documentation
patch had an unmatched context and applied no changes; it was corrected after
rereading. No product guard, assertion or lint was weakened.

| Accepted check | Result |
| --- | --- |
| New full/minimum CLI behavior |9/9 PASS |
| Tunnel regression |122 PASS/1 existing browser ignored |
| Minimum CLI all-targets |PASS |
| Original CLI stack checks |5 PASS |
| Strict CLI Clippy/format |PASS/PASS |
| Section11 full Rust/Python split |2590 Rust/13 Python,0FAIL/2existing ignored |
| Project .venv tool tests |73 PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

CLI985/Core1443/public52 PASS. Workspace fmt/check/strict Clippy/MSRV,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI,Python3.12
wheel-sdist build/install/import,tooling73/help118/raw38/host5/CLI95/backup3 PASS.
Full uses the prescribed system-Python dispatcher and selected project binding
interpreter; the extra73 tool tests ran directly with project `.venv/bin/python`.
22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN; maturity unchanged.

Frozen patch SHA256:
`995001651bbd61e410eb00803cddd765ce99bb067cfac01b4fa3470bb9bd8c3f`.
Actual debug ios SHA256:
`a5d0be4a0f7246e098eaaa8397268ac7d48768bb9828b8e638a83bdd64a57fc4`.
The committed patch matches the tested freeze. Ignored logs, exit codes, hashes,
source copies and review: `target/validation/tunnel-shutdown-admission-2026-10-06/`.
Reference HEAD/status unchanged; no cache clean, push, publication, history
rewrite, reference edit or subagent.

Coordinator lock wait itself still awaits release. Its total/cancellation,
post-check races, configuration/startup and whole-scan budgets, metadata bounds,
existing forwarding/delegated Axum/Core joins and other L01/L02 remain. This is
not an atomic guarantee against every concurrent post-check publication, async
Drop or physical freshness/cleanup/resource/performance certification. Continue
those independent tasks, L03–L04 and classes5–9. The known UID resource finding
remains unfixed and other tasks retain their original acceptance conditions.
