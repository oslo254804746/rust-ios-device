# Packet forwarding watch cancellation

Implementation: `a4fc6aa6ccf455773009c3a1a38a9d0e8ca67958`; parent `028647e3801d4f098fa81da22d3ee087c4856ee5`.
Linux x86_64, 2026-10-07, Rust 1.97.1/MSRV 1.80.1, project Python3.12.13.
Goal remains all30 main tasks and subtasks excluding Q04:9complete/21unfinished
(9notstarted/12inprogress-or-conditions). This profile advances L01.

The original forwarder watched cancellation while selecting a branch. Once a
branch won, TUN write, stream write and flush could wait without observing that
watch. A live owner could therefore stay blocked after a closed sender or a
changed event, even though caller-future drop already aborted its owned reader.

Each iteration now uses a biased outer watch selection around its complete
inner branch selection and selected write/flush. Private ForwardStep conveys
Continue,End or the original error to the loop; all exits still request reader
abort_all. One receiver is retained. Ready cancellation wins before new work;
an error acquired by the current step in the same poll remains the primary.
Complete-frame queues, MTU validation, active framing and reader ownership stay.
Whole-loop outer-only cancellation would skip checks when repeated Ready work
continues in one poll; the implementation rebuilds the check each iteration.

Two Core files278+/27-,six new tests. The prior375-line/11723-byte test prefix
(seven reader ownership cases) remains byte-for-byte. All older assertions stay;
active framing/MTU logic is unchanged. No public API/error variant/dependency/
policy/pairing/binding change.

The six actual public-forwarder fixtures exercise closed sender at a controlled
TUN-write wait, actual capacity1 duplex stream-write wait and controlled flush;
send(()) at a pending TUN write while keeping the sender alive (receiver1→0);
all-Ready work closing the sender during its first read (one read/40-byte write);
and controlled TUN IO closing the sender while returning a typed std::io::Error
with BrokenPipe (original object/class Disconnected retained). Flush/TUN waits
are controlled conditions; duplex flush itself is always Ready, and the typed
IO does not establish a real OS failure. Stream-write evidence drains0x60 before
EOF; flush/Ready cases drain the complete40-byte packet.

finish_watch stores the production completion/observation-timeout result before
dropping work and observing/recovering the stream/task baseline. Regression
assertions happen after recovery; subsequent caller drop cannot make a failed
watch-completion check pass. One-second fixture observations are host test limits,
not a new product deadline, CPU preemption or device ACK promise.

Old-body control71973 exited101:2PASS/4FAIL. Three close write/flush cases and
the changed-send case exceeded their fixture wait; acquired-IO and ready-loop
cases passed in that run. The original select was unbiased, so this does not
claim its busy-path cancellation always wins. Existing reader owner remained.
Wrong whole-loop shortcut control90513 exited101:0PASS/1FAIL; the next source
read returned the finite marker BrokenPipe instead of successful stop. Both
controls completed cleanup before assertions; no production hook was added.

The first frozen pipeline52321 exited1 at validation failure-path tooling:
the new test-only ForwardingFault derived Debug, violating the owned-error
safe-Debug gate. Its exact frozen source, logs and failure remain archived in
failed-run-52321. After that process exited, Debug became a fixed payload-free
manual implementation; the unchanged diagnostics verifier passed7 tests. The
forwarding code and six behavior assertions stayed unchanged from the controls.

Corrected fix, frozen pipeline52489 terminal0/all10 serial gates. Source hashes
checked between gates/end. lifecycle_audit reviewed actual diff/fixtures/Tokio
read-only; reconnect_design independently investigated the next L03 scope. Only
the primary applied product changes and ran Rust tools, with source frozen.

| Check | Result |
| --- | --- |
| Six new full/minimum tunnel-feature tests |6/6 PASS |
| Core tunnel regression including prior reader tests |59 PASS |
| Minimum Core tunnel/CLI all-targets |PASS/PASS |
| Original CLI stack/strict Core Clippy/format |5 PASS/PASS/PASS |
| Section11 complete validation |2644 Rust/13 Python;0FAIL/2existing ignored |
| Project .venv tooling |73 PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

CLI1025/Core1457/public52 PASS; workspace fmt/check/strict Clippy/MSRV1.80.1,
66features/fourgroups/7+7doctests/three snippets/C ABI/Python3.12wheel-sdist
rebuild-install-import/tools73/help118/raw38/host5/CLI95/backup3 PASS.
22device NOT_RUN;Chromium/installedPython3.9/3.13 NOT_RERUN. No maturity upgrade.

Patch SHA256: `ad4fcdf98289f55115fc2996de42b6ae5d83075ffc10c916e8df1e6ef88896e3`.
Actual debug ios SHA256: `5026030b49bf6787ab84c7947843ab0833f253b9724b94034fa4fa080b104cf6`.
Committed patch matches freeze; ignored source/control/gate logs/exits/hashes/
reviews in `target/validation/forward-watch-shutdown-2026-10-07/`.
Other product/Q04 sources and reference HEAD/status remain unchanged.

Cancellation may abandon partial writes and terminally close the stream; no
rollback/ACK guarantee. CPU inside one poll and blocking Drop are not preempted.
Already-acquired step IO is retained; unconsumed worker-queue errors may be skipped
when cancellation is ready. Normal reader joins/panic propagation/cleanup deadline,
outer kernel/whole native owner joins, other startup/whole-scan/mutations/metadata/
physical freshness/reuse/restart/manual epochs and L01-L04/classes5-9 continue.
No OS TUN/device/socket/RSS/long-run/performance/platform certification. Q04 and
all subtasks remain excluded,UID unfixed,other acceptance unchanged;goal active.

Documentation review passed for all seven changed files and 389 local links.
The 118 capability definitions, maturity levels, previous evidence and roadmap
history remain intact; only the current L01 row and an appended delivery record
changed. The committed implementation matches the frozen patch, all 436 product
hashes, the debug binary and both reference checkouts. The catalog checker passed
against that binary without rebuilding it.
