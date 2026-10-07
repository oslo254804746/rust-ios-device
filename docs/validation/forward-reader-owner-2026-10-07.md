# Packet forwarding reader ownership

Implementation: `a455d6788168b61b3bae49fa0a996b3fa2b5824e`; parent `0981ee2e0e8c0b7af352abf4c236d9c5d59ee78d`.
Linux x86_64, 2026-10-07, Rust 1.97.1/MSRV 1.80.1, project Python 3.12.13.
Effective goal: all 30 main tasks and subtasks excluding Q04. Nine complete,
21 unfinished (9 not started, 12 in progress or awaiting conditions).

The packet forwarder spawned a reader and kept a bare JoinHandle in its future.
Dropping that future detached the reader. A partial IPv6 frame or an open idle
peer could leave it retaining the stream after its caller had gone away.
This also happened when the parent was cancelled during a TUN/stream write or
flush, or when a parent task panicked. A closed packet receiver did not help
while the reader remained inside read_exact waiting for more bytes.

The private implementation now owns its one reader in a local JoinSet. Caller
drop, abort or unwind drops the set and requests reader abortion. Normal
forwarding still uses the original select, complete-frame queue and outcome;
normal return requests abort_all and returns the same result without awaiting
the reader. The framing comment now separates active select competition from
terminal cancellation, when dropping a partial frame is intentional.

Two Core files add 387/remove 4 lines, including seven regression tests.
All existing forwarding branches, framing/queue/MTU rules and old assertions
remain byte-for-byte. No public API/error variant/dependency/protocol/pairing/
policy/binding or userspace proxy ownership change. This does not replace the
existing userspace proxy JoinSet or attach an owner to the outer kernel task.

Fixtures use actual Tokio DuplexStream IO with a tracked wrapper, default
current-thread runtimes, native JoinError and bounded observations:

- Direct future drop after consuming 20 bytes of a 40-byte IPv6 header.
- Direct drop while the controlled TUN write waits after one complete packet.
- Actual duplex capacity 1 stalls write_all after one byte. The peer drains
  and verifies `0x60` before observing EOF; it does not assume the first read is EOF.
- A controlled transport poll_flush waits after all 40 bytes were written.
  Duplex flush itself is always ready; this is not an OS flush fault.
- Parent task abort is awaited and retains a native cancelled JoinError.
- Parent TUN-write panic is awaited and retains the actual TunPanic payload.
- Normal watch sender closure keeps Ok during a partial-header read.

Cancellation cases keep the peer open while observing stream release, EOF and
task count relative to their isolated runtime baseline. The fixture then drops
the peer and waits for exactly one stream Drop plus baseline task recovery
before asserting the captured observations. This also recovers old detached
readers before negative-control failures. These are resource observations,
not an explicit join of the reader or a wall-clock performance guarantee.

Old-body control 40930 exited 101: 1 PASS/6 FAIL. All six caller drop/abort/unwind
cases failed at the captured stream-release assertion after recovery. Normal
watch closure passed, as its old normal branch already requested reader abort.
Original spawn/abort behavior was restored for the control; new tests/module
and the revised comment remained. There was no new production hook or helper.

The fix was restored and formatted, then frozen pipeline 96089 exited 0 with
all ten serial gates. Product hashes were verified between gates and at end.
The subagent reviewed actual source, fixtures and local Tokio implementation
read-only; the primary alone applied source and ran every Rust command.

| Check | Result |
| --- | --- |
| Seven new full/minimum tunnel-feature tests |7/7 PASS |
| Core tunnel regression |53 PASS |
| Minimum Core tunnel/CLI all-targets |PASS/PASS |
| Original CLI stack cases |5 PASS |
| Strict Core Clippy/format |PASS/PASS |
| Section 11 complete validation |2638 Rust/13 Python;0 FAIL/2 existing ignored |
| Project .venv tooling |73 PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

CLI1025/Core1451/public52 PASS. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four groups,7+7 doctests/three snippets,C ABI,Python3.12 wheel/sdist
rebuild-install-import,tools73/help118/raw38/host5/CLI95/backup3 PASS.
22 device NOT_RUN;Chromium/installed Python3.9/3.13 NOT_RERUN. No maturity upgrade.

Frozen patch SHA256: `45cacb4cf65201e5b934c62ce92ab3acd3a71d1adf90b889e83453a5924f877d`.
Actual debug ios SHA256: `c7b9351f4c47680f731be9e33e8b782700479d91c6da818102ba548f79a2f2b5`.
Committed patch matches the freeze. Ignored source copies, original control,
gate logs/exits/hashes and reviews live in
`target/validation/forward-reader-owner-2026-10-07/`.
Other product/Q04 sources and read-only reference HEAD/status remain unchanged.

The evidence supports scheduled abort and eventual stream/task release on
cooperative execution. Normal reader joins, reader panic propagation, an
independent cleanup deadline and outer kernel/whole native owner joins remain
pending. CPU within a poll or a blocking destructor is not preempted. Startup,
whole-scan/other mutations/metadata/physical freshness/reused IDs/restart/manual
epochs, other L01/L02/L03-L04 and classes5-9 continue. No device ACK, socket/RSS,
long-run, throughput or platform certification. Q04 and all its subtasks remain
excluded, the UID finding stays unfixed and other acceptance conditions remain.
The complete goal remains active.
