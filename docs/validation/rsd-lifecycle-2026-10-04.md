# Borrowed RSD bootstrap and read ownership

L01/class 4 host slice accepted 2026-10-04, source
`85b8621f069b164d7da251a9531faf43b3456691`, baseline
`2088e739f04134e60341346cfdb1d4ca93e86f8d`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. One source/test file, 543 insertions/2 deletions.
[Lifecycle contract](../lifecycle-contract.md), [public compatibility](../public-contract.md),
[error contract](../error-contract.md).

The internal borrowed RSD helpers now retain logical ownership independently
of each H2 operation. Initialization/queued bootstrap commits after its first
complete write. Raw stream-1/stream-3 reads commit when taking a 24-byte wrapper
header, including one buffered by an earlier H2 operation. Discovery retains
completed messages while skipping non-handshake replies with the existing
six-message parser policy. An empty subsequent read cannot hide these earlier
commitments from cancellation.

Cancellation or returned failure after commitment, or lower H2 invalidation,
invalidates the borrowed framer through its existing invalidator and clears
H2 buffers/queues/accounting. Further cached/wire operations reject. Original
operation errors stay owned and unchanged; later rejection is the existing
H2-to-XPC Protocol family. No XpcConnection pending map exists in these helpers.
Empty idle/unpolled operations and normal completions preserve local framing.
A complete stream-local error before commitment can preserve other streams;
a failed committed initialization requires a fresh transport.

Existing dial/bootstrap/response/outer timers, fresh-connection fallback order,
wire bytes, flags, selectors, directory parsing and discard-reader acceptance
remain. There is no new RSD timer/zero-budget API or public export/signature,
frontend option or error variant. Helper futures are boxed before constructing
their await state and remain directly owned, without a task or reader queue.

Separately completed queue/send/read stages are not automatically combined
into a new request budget. Owned discovery attempts already drop their transport
on failure/cancellation. The single-write host announcement retains its H2
guard. Manual-pairing bootstrap uses guarded initialization; later direct
protocol exchanges remain a separate owner boundary. Transport remains owned
until Drop; raw borrowed transport owners must not bypass invalid state. There
is no remote ACK, cleanup completion, rollback, safe retry or peer-liveness
guarantee from local framing state.

| Owned host fixture | Evidence / boundary |
| --- | --- |
| Empty/unpolled/pre-send rejection | Unpolled initialization/queue/read emits no bytes; empty discovery cancellation stays usable, as does existing internal H2 zero-budget rejection before the first send; subsequent ACK/valid directory succeeds |
| Sent initialization | First request fully sent, response withheld; caller cancellation and actual stage expiry invalidate. Complete late directory cannot satisfy later helpers, no second outgoing request, peer waits until framer Drop then gets EOF |
| Buffered wrapper | Earlier H2 stream-5 read buffers the complete wrapper on stream 1 or 3. Body wait has no new transport progress; caller cancellation/expiry invalidates, rejects late body and a cached stream-5 byte |
| Queued bootstrap backpressure | Duplex capacity equals HEADERS1 plus the first DATA1. The next HEADERS3 write blocks before its first byte; dropping the queue future invalidates despite an empty current H2 write. Exact emitted prefix and later EOF are checked |
| Complete stream reset | RST_STREAM before message commitment keeps stream 3 readable; the same received reset after the first initialization send invalidates. Original typed H2 cause survives |
| Skipped/malformed messages | Cancellation/actual expiry after a complete skipped ACK invalidates discovery; existing wrapper decoder and oversized declared length reject and invalidate after header consumption |
| Successful completion | Both initialization and queued bootstrap preserve exact frame order/payloads. Historical non-decoded discard ACK acceptance remains, followed by a valid directory and another complete host announcement on the same framer |

Seven internal tests added; all-features and minimum tunnel each run seven.
New peer work uses owned joins/pinned futures with one-second fixture bounds,
without new fixture tasks. XPC124 regressions and five existing CLI 16-KiB stack
budget tests pass. Existing public consumer tests run in the full gate. No
old-source negative control, throughput measurement, actual device bootstrap,
remote cleanup or platform certification was performed.

The first narrow compile failed E0061 because an existing test calls the original
private read entry. Keeping that signature and moving its guarded body into an
inner helper fixed it; old tests/assertions were not changed or weakened. Final
narrow/minimum/regression/stack/strict Core+CLI Clippy/fmt pass. Source audit
compares original bootstrap/raw bodies after removing ownership markers and
keeps all old tests, retry policy, parser, chunked reader, XpcConnection/budgets,
H2, codecs, classifiers, frontends, dependencies and reference checkouts intact.

First full gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session91844 terminal0, validation/full PASS: 2182 Rust/13 Python, zero failures,
two existing ignored; CLI706/Core1323/public43. Workspace fmt/check/strict Clippy,
MSRV, 66 feature checks/four test groups, 7+7 doctests/three downstream snippets,
C ABI, 3.12 wheel/sdist/install, 53 tooling/118 actual-help/38 raw/five host/
15 CLI/three backup contracts pass. Chromium and installed 3.9/3.13 were not
rerun; 22 device rows remain NOT_RUN. Capability maturity stays unchanged.

Frozen source SHA256:
`bedc6cff1eb4bbd50c5539416cc877c10a6f203644bf78d82677f1c6ff614341`.
Final ios debug binary SHA256:
`685fee93dad137cd7b89c0c66384539ca5742c67a0831cefe31062f8f9110027`.
Source stayed frozen through the terminal full result. No clean/storage failure;
about 7.7 GiB remained. Logs `/tmp/ios-l01-rsd-` are local, while this topic
records durable outcomes. Preparatory session88918 terminal101, corrected
narrow19913 terminal0 and remaining narrow pipeline53566 terminal0 are separate
from the passing full evidence.

L01/class4/all31 goal remains active. Lockdown client/device-session owners,
DeviceLink/plist, later manual-pairing exchanges, other split requests/total
budgets/cleanup/long streams, L02–L04/Q04 and platform/device conditions remain.
No device, credential, Apple-side operation, reference edit, push, publication
or history rewrite occurred.
