# Manager resource baseline host acceptance

The interrupted L04 Linux manager/native **host subitem** is accepted on
2026-10-09. L04 remains incomplete: payload/file throughput, service streams,
physical iOS devices and other platforms still require their original acceptance.
[Runner and reproducible workload](../manager-resource-baseline.md).
[Recorded comparison and diagnostic evidence](manager-resource-baseline-2026-10-09.json).

## Comparable performance runs

Before production revision: `2964c1c2109b0368237e9ebaf13f99e3cbb0da13`.
After production revision: `e53e0bb8b2dc5f5c1e61260cfde6b345c1f591b8`.
Both received the identical test-only fixture and parent module registration;
fixture SHA-256 `5a81963d96062f39dffd77d980c669b57b767a5737e6d47c27cc391a81addfc5`.
The driver, compiler and inputs also match. Serial all-features/locked debug
builds used rustc 1.97.1, `CARGO_INCREMENTAL=0`, the shared target directory,
Linux x86_64 kernel 7.0.0-34-generic, an AMD Ryzen 5 5600G VM with eight logical
CPUs. Governor, affinity and concurrent load were not measured.

Each scale has five rounds; the separate 16-device long run completed 104 rounds
in approximately 30 seconds on each revision. Each round creates fresh original
native owners. HTTP DELETE alternates with real native-peer FIN in the long run.
Original native/proxy joins and strict EOF, empty registry, recovered permits,
HTTP server join and actual Tokio task recovery all passed.

Scale observations in milliseconds; every cell is p50 / p95. HTTP values are
per device; native cleanup covers the entire group.

| Devices | POST before / after | GET before / after | DELETE before / after | Cleanup before / after |
| --- | --- | --- | --- | --- |
| 1 | 1.172 / 1.325 → 1.122 / 1.413 | 0.568 / 0.610 → 0.572 / 0.605 | 0.582 / 0.599 → 0.644 / 0.765 | 0.448 / 0.523 → 0.478 / 0.497 |
| 4 | 0.973 / 1.324 → 0.973 / 1.259 | 0.549 / 0.649 → 0.574 / 0.621 | 0.584 / 0.674 → 0.652 / 0.687 | 0.534 / 0.892 → 0.555 / 0.591 |
| 16 | 0.920 / 1.085 → 0.946 / 1.085 | 0.578 / 0.641 → 0.583 / 0.621 | 0.582 / 0.661 → 0.659 / 0.734 | 0.917 / 1.207 → 0.890 / 0.910 |

The slower DELETE observations are retained. Two runs on an uncontrolled host
do not establish statistical regression or improvement. Proposed host-only
investigation thresholds, derived by doubling the largest measured scale p95
and rounding upward to 0.1 ms, are POST 2.9 ms, GET 1.3 ms, DELETE 1.6 ms and
group cleanup 2.5 ms. These are provisional comparison thresholds for this debug
fixture, with no physical-device or production latency promise.

## Growth attribution

Original released long-run RSS grew 344,064 bytes before and 315,392 bytes after.
Both second-half ranges were 57,344 bytes. Released FD count remained 13,
socket FD count 6, Tokio tasks 5, retained coordinator cells 16 and occupied
registry slots 0. At final teardown, tasks returned to the pre-manager value 0,
FDs to 10 and sockets to 3. Counts alone do not explain RSS.

Separate diagnostics reused the exact archived binaries with a Linux/glibc
preload probe. They ran for approximately 60 seconds / 208 and 207 cycles,
respectively, retaining all normal join/EOF/cleanup assertions. The probe adds
one thread and one file, samples all allocator arenas and Linux RSS categories
every 500 ms, and tries `malloc_trim(0)` only after the test has exited.
These timings are separate from the uninstrumented performance comparison.

| Diagnostic | Before | After |
| --- | --- | --- |
| RSS range at 50–60 seconds | 64,401,408–64,401,408 bytes | 60,346,368–60,346,368 bytes |
| Anonymous RSS at 50–60 seconds | 5,595,136 bytes | 5,750,784 bytes |
| File-backed RSS at 50–60 seconds | 58,806,272 bytes | 54,595,584 bytes |
| Allocator non-free estimate after fixture/runtime teardown | 50,168 bytes | 50,168 bytes |
| Arena free bytes before final trim | 912,392 bytes | 687,112 bytes |
| RSS released by final trim | 20,480 bytes | 28,672 bytes |

The large process RSS is predominantly resident executable/library pages.
After the first ten seconds, file-backed RSS is constant in the periodic samples;
the remaining gradual RSS increase is anonymous memory. It reaches a sampled
plateau in the final ten seconds. Allocator accounting shows cycling allocations
and retained free arena space, while fixture/runtime teardown reduces the
non-free estimate to the same 50,168 bytes on both revisions. Final trimming
releases additional anonymous pages without changing that estimate or file RSS.
This attributes the observed bounded warm-up to process/allocator retention
rather than an accumulating native owner registry. It does not identify every
Rust allocation, prove zero leaks or certify unlimited-duration stability.

[`malloc_info`](https://man7.org/linux/man-pages/man3/malloc_info.3.html) reports
all arenas. The estimate is aggregate arena system bytes minus reported free
bytes plus mmap bytes; it includes overhead and tcache and is not exact live
application memory. [`malloc_trim`](https://man7.org/linux/man-pages/man3/malloc_trim.3.html)
attempts to return free heap pages. No trimming was added to production code.

## Verification and retained evidence

At host acceptance, the previous serial builds and strict CLI Clippy had
succeeded and their 513 frozen product paths were verified byte-identical. The exact accepted binary hashes, source
manifests, toolchain, test overlays, raw logs, full ordered samples and certificates
remain under `target/validation/manager-resource-baseline-2026-10-09/`.
The comparison was rerun successfully with artifact-hash and receipt checks.
Current format verification and strict C probe compilation also passed.
The tracked JSON preserves the results and hashes independently of build caches.

An initial diagnostic attempt inherited the preload into a host-metadata helper,
which created the output before the test; the test exited 90 and produced no
receipt. Its failed evidence is retained as `allocator-before/`. The probe now
checks `/proc/self/exe` against the exact admitted native binary; corrected runs
passed. A preliminary successful probe was subsequently extended with separate
anonymous/file RSS fields; final acceptance uses the `*-pages` runs.

Python/C query packages and Darwin watch remain separate unfinished work.
This acceptance completes the interrupted measurement/attribution batch only.
