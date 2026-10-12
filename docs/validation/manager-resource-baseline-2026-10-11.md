# L04 resource and workload delivery — 2026-10-11

The original L04 development scope is implemented: reproducible 1/4/16-device
host measurements cover connection/query/stop, native ICMP transfer, real AFC
disk-file transfer and timed Syslog streams, with p50/p95, throughput, sampled
memory/CPU/tasks/handles/TCP endpoints and original-owner cleanup. This closes
development delivery under the user's rule that missing platform/device runtime
verification remains separate. It does not certify physical hardware or promise
a throughput improvement. The exact observations and reference limits are in
[the structured report](manager-resource-baseline-2026-10-11.json).

## Comparable runs

Before: `af20f6aa3752d2511b763b14239cda78a1db9bef`, with the identical test-only
benchmark overlay. After: `c98c819153e5fe3f12f4c31845f6026f089a7492`.
Both use rustc 1.90.0, Windows MSVC debug/all-features, Windows 11 build 26100,
16 logical CPUs and the same reported Intel processor fields. The fixture,
driver, child-module hashes, inputs and build mode match. Exact baseline overlay,
frozen source evidence, binaries, original logs and run receipts remain in the
repair artifact directory. This comparison predates the next development batch.

Inputs: one scale sample at each of 1/4/16; 16 long-run cycles, 30 s minimum,
250 ms pauses, 256 KiB payloads, four disk files per AFC operation, and actual
500 ms Syslog windows. Actual long runs lasted 162595/163016 ms. All 272
16-device measurements per workload are retained; no warm-up sample is discarded.
The one-device scale has one sample and cannot establish a reliable tail latency.

| Measurement at 16 devices | Before | After |
| --- | ---: | ---: |
| Native ICMP throughput p50, MB/s (decimal) | 7.5805 | 7.6622 |
| AFC file throughput p50, MB/s (decimal) | 13.7622 | 13.5213 |
| Native ICMP operation p95, ms | 52.2645 | 54.1861 |
| AFC operation p95, ms | 62.5616 | 63.5295 |
| Manager GET p95, ms | 0.7766 | 0.7728 |
| DELETE/stop p95, ms | 1.0984 | 0.9890 |
| Native cleanup p95, ms | 1.2487 | 1.4760 |
| Disconnect-to-absent p95, ms | 10.3539 | 11.1731 |

Every workload is pinned once and sampled concurrently at a nominal 10 ms
interval until that same future finishes. Maximum observed gaps are recorded.
Sampling never cancels a partial frame or Syslog read; the fragmented Syslog
fixture crosses sampling ticks and still decodes the original complete message.
Peaks are sampled process extrema, including fixture peers. OS peak RSS is a
lifetime high-water mark and is not relabelled as a per-operation peak. Windows
handles and owner-PID TCP rows are separate counters; Unix FD counters are null.

Reference budgets are empirical: baseline maximum elapsed time and minimum
observed throughput, with explicit flags for an after-run p95/rate outside that
range. Several 1/4-device values exceed the observed reference. The 16-device
ICMP/AFC p95 values remain inside baseline maxima, although their p95 increased.
These flags require investigation when reused as a performance gate; they are
not silently converted into PASS and no arbitrary tolerance is added.

## Resource attribution

All 16 released phases have handles 154, own TCP endpoints 3, Tokio tasks 5,
native occupied 0, empty manager records and fully restored permits. Both stop
and actual peer disconnect retain original task joins, strict EOF and original
native failure causes. Final HTTP/native owners join; tasks recover to zero.
The retained 16 bounded per-device coordinator entries stay constant throughout.

The comparable after run's private commit increased 962560 bytes between first
and last released samples but rose and fell throughout. Its private working set
increased 1052672 bytes. Non-private working set increased only 28672 bytes and
has zero second-half range. Thus the approximately 96 MB non-private resident
component is already warmed and stable, rather than growing native owners.
The remaining private-memory variation prompted a separate allocator diagnostic.

The diagnostic uses the immutable c98 production test executable with a
test-only default-process-heap observer, the same workload inputs and 16 cycles
(165511 ms). All original cleanup checks passed. All 16 HeapWalk observations
ended with ERROR_NO_MORE_ITEMS and successful unlock. No allocation, log or await
occurs while the default heap is locked; foreign heaps are never walked.

| Released-phase diagnostic | First → last | Observed range |
| --- | ---: | ---: |
| Default-heap busy bytes, including block overhead | 661744 → 665984 | 4384 |
| Default-heap region commit bytes | 4775936 → 4800512 | 917504 |
| Process private commit bytes | 8048640 → 7970816 | 774144 |

Region commit and private commit rise/fall together (Pearson correlation
0.99276), while busy blocks remain nearly constant. This supports allocator
capacity variation as the explanation for these observations. It is an
inference, not an exact causal proof for the earlier run or a universal leak
certificate. LFH free space is not individually reported; region commit minus
busy bytes is not asserted to equal free memory. Other heaps, stacks, mappings
and fixture allocations remain outside this particular attribution.

The diagnostic executable hashes match before/after. Its pre-run fixture hash
is retained. A cfg-only parent-source adjustment happened after execution
started, so the diagnostic does **not** claim a whole-workspace source freeze.
It is kept separate from the frozen before/after comparison. The read-only
`manager_resource_attribution.py` validates original receipts, hashes, complete
heap walks and cleanup before producing the diagnostic report.

## Remaining runtime coverage

Physical USB/Wi-Fi devices, release builds, other host platforms and performance
under controlled CPU frequency/affinity/background load remain NOT_RUN.
The baseline measures the stated native ICMP/AFC/Syslog paths; it is not evidence
for every TCP/DTX service. Additional paths can use the same tooling as needed.
L01/L02 implementation gaps and other original tasks remain open.
