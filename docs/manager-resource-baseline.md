# Manager resource and latency measurements

This is an explicit L04 **host subitem**. It measures the authenticated production
manager router, `Source::connect_owned`, the original native retirement registry,
and real `UserspaceTunDevice` workers for 1, 4 and 16 independently owned devices.
Device discovery and record metadata use the CLI-private test boundary. Every
device owns its own real loopback TCP native peer and userspace listener; the
metadata wrapper delegates liveness and native identity to that original handle.

The ordinary workspace test run ignores this gate. An ignored count or a newly
compiled test is not execution evidence. Before acceptance, run the gate explicitly
against both production revisions with the **same fixture overlay**, compare the
recorded results, and attribute any resource growth.

## Deliverables and remaining conditions

The fixture is
`crates/ios-cli/src/cmd/tunnel/tests/native_retirement_tests/resource_baseline_tests.rs`.
The reproducible runner and comparison entry point is
`scripts/manager_resource_baseline.py`; it uses only an existing compiled native
test executable and never calls Cargo, rustc, maturin or a build script.

This host fixture records HTTP start/query/stop latency, native cleanup latency,
strict original TCP peer EOF, actual Tokio task counts, manager registry and
permit usage, and Linux process RSS/file descriptor/socket samples. It alternates
HTTP DELETE with actual write-side FIN on the original native peer, which makes
the native reader acquire its original `UnexpectedEof`. The native error remains
an error in the receipt; only the original typed join certificate can prove its
joins complete. Stop tests require a clean original native shutdown. A reset,
timeout, unexpected byte, liveness bit, or task abort alone cannot prove EOF or
native cleanup.

Payload throughput, AFC/file transfer, other long-lived service streams,
representative physical iOS devices, USB/Wi-Fi routing, OS TUN, and macOS/Windows
resource measurements remain **NOT_RUN**. This subitem cannot complete L04. It
does not establish real-device RSD/service behavior or an operational performance
promise.

## Main-agent serial preparation

All Rust actions belong to the main agent's one-job queue. First inspect actual
existing build/validation processes; do not terminate another session's process.
Use isolated worktrees for the production baseline and current implementation.
Add only this identical new test file plus the parent registration
`mod resource_baseline_tests;` to each tree. Retain the exact test overlay patch
and source manifest. The baseline production revision did not originally contain
this benchmark, so evidence must distinguish the production commit from its
test-only overlay.

The main agent compiles each tree serially with the same pinned toolchain,
features, lockfile and build profile. A suitable explicit build action is:

```bash
cargo test -p ios-cli --bin ios --all-features --locked --no-run --message-format=json
```

Select the `compiler-artifact` executable where `target.name == "ios"` and
`profile.test == true`. Preserve that exact path/hash or copy the executable to
an evidence directory before compiling the next tree into a shared target dir.
Never infer the executable from the latest glob match. Retain compiler `-Vv`
output collected by the main agent as a file; the runner requires that existing
evidence and does not execute the compiler. A custom profile must be declared
`custom`, with its exact Cargo configuration archived by the main agent.

Freeze the tested source tree for compilation/execution, including the test
overlay and this driver. Keep the frozen manifest and the root validation
runner's before/after verification beside the measurement results. Source
freezing and the global Rust-job queue are external main-agent responsibilities;
the measurement driver does not claim to establish them.

## Execute the explicit gate

Run one compiled executable at a time. The example paths below are local evidence
paths chosen by the main agent; use the actual archived executable and fixture
source for the tested production revision.

```bash
python3 scripts/manager_resource_baseline.py run \
  --test-binary /absolute/evidence/baseline-ios-test \
  --fixture-source /absolute/baseline-worktree/crates/ios-cli/src/cmd/tunnel/tests/native_retirement_tests/resource_baseline_tests.rs \
  --revision 7b91c24a3f85d9639208f2249744cb59db97f9b3 \
  --rust-version-file /absolute/evidence/rustc-Vv.txt \
  --build-mode debug \
  --label before \
  --output /absolute/evidence/manager-before \
  --samples 5 --duration-ms 30000 --min-cycles 16 --max-cycles 128 --pause-ms 250
```

Run the same command for the current production revision with a distinct output
directory and `--label after`; use its exact forty-character commit. Inputs,
driver hash, fixture source hash, compiler evidence and declared build mode must
match for a comparison. Debug/release declarations are checked against the
executable's `cfg!(debug_assertions)`; preserve profile evidence because that
flag alone does not describe optimization, LTO, debuginfo, or feature choices.

The command executes precisely this existing test, in the foreground:

```text
cmd::tunnel::tests::native_retirement_tests::resource_baseline_tests::manager_native_resource_baseline
```

Libtest receives `--exact --ignored --nocapture --test-threads=1`. No other ignored
test is accepted as a substitute. The dedicated test runtime has two Tokio worker
threads, uses an unpaused monotonic clock, binds only owned numeric loopback
addresses and reads an owned policy file granting exactly the sixteen fixture
IDs. Before measurement it checks that unauthenticated HTTP is rejected and
authenticated empty listing succeeds without acquiring any native device.

Default input is five rounds per scale, then at least 30 seconds and sixteen
completed long-run rounds, alternating the two termination paths. At most sixteen
native devices exist concurrently and the long run stops after at most 128 rounds.
If the admitted maximum expires before the requested duration/minimum rounds,
the fixture fails explicitly. Input bounds are 1–20 scale samples, 1–1,800,000 ms
duration, 2–256 minimum/maximum rounds with minimum at most maximum, and 0–5,000 ms
inter-round pause. These are fixture input bounds, not product performance budgets.

The fixture has an independent measurement total of requested duration plus
180 seconds and finite cleanup budgets. The external runner deadline is requested
duration plus 300 seconds. It retains at most 8 MiB of raw native output and 4 MiB
per JSON artifact. If an admitted larger input exceeds an artifact bound, the
runner fails and preserves the bounded raw prefix and execution metadata; it does
not silently discard samples and call the result successful. Timeout/overflow
kills and reaps only the runner's owned test process and cannot produce an accepted
native cleanup receipt. No detached benchmark or repeated background Rust task is
started.

## Sampling and interpretation

Each HTTP latency includes request execution through bounded response-body
consumption. Samples remain in execution order in `fixture.json`; aggregation
uses all samples and nearest-rank p50/p95 without silently discarding warm-up
samples. Setup/authentication warm-up is outside the reported scale/long-run
distributions. `post_start`, `get` and `delete_stop` are per-device observations;
`disconnect_to_absent`, `native_cleanup` and `round_total` cover the entire device
group. Do not compare their sample counts or interpretation as interchangeable
per-device timings.

Native cleanup samples include the production retirement drain, original handle
shutdown, original native/proxy strict EOF, and registry/permit recovery checks.
The drain may return the retained original EOF in a disconnect round; clean stops
must return success. Every round retains the exact native handle through join and
EOF observation. Successful receipts require fresh owned publication for every
device and check pointer identity between production manager ownership and that
original handle. There is no synthetic task/liveness counter standing in for a
native runtime.

Resource sampling uses actual
`Handle::current().metrics().num_alive_tasks()` and Linux `/proc/self/status`
(`VmRSS`, `VmHWM`, `Threads`) plus `/proc/self/fd` targets. A target beginning
`socket:[` is a socket file descriptor. The open sampling directory itself is
included in the FD count. The directory scan is an observation over a changing
process, not an atomic OS snapshot. Missing/unsupported samples are reported as
unavailable, never zero. Other platforms need their own sampler and validation.

The active sample describes the **whole test process**: real production workers
plus owned fixture native-peer sockets, idle proxy-client sockets, authenticated
HTTP connections and fixed runtime/harness overhead. Each proxy sends no bytes;
the real listener admits its child, and the original join report must contain at
least the four original workers plus that child before the active sample. This
does not measure payload throughput. Closed original handles and sockets are
dropped before the released sample.

The harness drives authenticated POST/GET/DELETE sequentially and keeps all
selected devices active together for each scale's active sample. It serves the
production router in an owned Axum server; it does not run the production
background discovery monitor. Concurrent establishment throughput and repeated
automatic-discovery scheduling need separate measured workloads.

To avoid confusing instrumentation growth with runtime retention, the measured
process emits bounded ordered events and retains only one round of JSON. The
Python driver aggregates them after the native process exits. Vector capacities,
allocator arenas, HTTP keep-alive state and the bounded manager's sixteen retained
device cells can still produce a warm-up plateau; the result must show that
plateau rather than assuming it. `round_total` includes resource/event collection
overhead. Peak RSS is inherently nondecreasing and cannot demonstrate live memory
recovery by itself.

The driver reports released-resource first/last/min/max, first/second-half maxima,
and second-half range. Positive last-minus-first is flagged for attribution.
It does not subtract guessed harness RSS, invent a leak tolerance, classify a
plateau automatically or announce latency/throughput targets. Compare the actual
RSS, FD/socket and task histories with original join/EOF receipts, explain growth,
and preserve unresolved attribution in the task list. Final acceptance requires
the production registry closed and empty, all manager permits recovered, the
original HTTP server joined and actual Tokio tasks returned to their initial
count. Expected native disconnect faults remain recorded after recovery.
The final process sample still includes the live manager shell, its fixed registry
and device cells, policy metadata and fixed evidence-buffer capacities. Compare
steady released samples to attribute that retained shell and allocator behavior;
do not equate native task/handle recovery with returning process RSS to the bare
runtime's pre-manager value.

## Compare and record evidence

```bash
python3 scripts/manager_resource_baseline.py compare \
  --before /absolute/evidence/manager-before \
  --after /absolute/evidence/manager-after \
  --output /absolute/evidence/manager-comparison.json
```

Every run directory contains `run.json` (host, compiler, production commit,
binary/driver/fixture/log hashes and execution), `native-test.log` (bounded actual
libtest stream), `fixture.json` (all ordered samples/certificates), and
`summary.json` (distributions and resource growth observations). The comparison
records both runs and p50/p95 changes with actual sample counts; it requires
successful receipts and input/toolchain/fixture parity. Host fields report CPU
model/count, machine/product, kernel, build declaration and memory metadata when
available. Frequency/governor, affinity and concurrent load remain explicitly
unobserved unless the main agent adds independent evidence.

Use this results template after actual execution; fill measured numbers from the
JSON artifacts and retain unresolved conditions:

| Evidence | Recorded value |
| --- | --- |
| Baseline production commit / identical fixture SHA | Actual `run.json` values |
| Current production commit / identical fixture SHA | Actual `run.json` values |
| Frozen source manifests / test-overlay patches | Exact archived paths and SHA-256 |
| Toolchain / profile / features / host / CPU / kernel | Actual compiler, build and host evidence |
| Input / actual scale samples / long-run duration and rounds | Actual `fixture.json` values |
| Start / GET / DELETE / native cleanup p50 and p95 at 1/4/16 | Actual summary values; preserve metric scope |
| Released RSS / FD / socket / actual Tokio tasks over time | Actual histories and growth attribution |
| Original joins / strict EOF / registry / permit / HTTP recovery | Actual receipts; failures stay failures |
| Performance budget proposed from baseline | Pending until reviewed measured baseline |
| File transfer and payload throughput | NOT_RUN in this fixture |
| Representative physical-device / remaining platform conditions | Exact outstanding prerequisites; L04 remains incomplete |

No main-task completion follows from adding this fixture or increasing the test
count. The engineering roadmap's original L04 acceptance remains authoritative.

## Optional allocator diagnosis

The [2026-10-09 host acceptance](validation/manager-resource-baseline-2026-10-09.md)
records the actual before/after comparison and growth attribution. To investigate
RSS separately, explicitly compile the Linux/glibc probe (no Rust compilation):

```bash
cc -shared -fPIC -O2 -Wall -Wextra -Werror -pthread \
  scripts/manager_allocator_probe.c -o /absolute/evidence/allocator-probe.so
python3 scripts/manager_allocator_diagnostic.py \
  --probe /absolute/evidence/allocator-probe.so \
  --test-binary /absolute/evidence/baseline-ios-test \
  --fixture-source /absolute/baseline-worktree/crates/ios-cli/src/cmd/tunnel/tests/native_retirement_tests/resource_baseline_tests.rs \
  --rust-version-file /absolute/evidence/rustc-Vv.txt \
  --revision 2964c1c2109b0368237e9ebaf13f99e3cbb0da13 \
  --label diagnostic-before --output /absolute/evidence/allocator-before
```

Repeat serially for the after revision/binary with a fresh output directory.
The diagnostic enforces 60 seconds, at most 256 cycles and normal original-owner
cleanup. It retains probe/driver/XML hashes, all snapshots, ten-second windows
and final before/after-trim observations. One added sampler thread/file and
allocator locks perturb the workload; never mix its latency with performance
evidence. The preload is admitted only into the exact test executable. Existing
LD_PRELOAD is rejected, absent RSS data fails, and incomplete probe lifecycles
cannot support attribution. Retain source freezes externally as for normal runs.
