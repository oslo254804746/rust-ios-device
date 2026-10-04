# XPC message, call and stream ownership

L01/class 4 host slice accepted 2026-10-04, source
`09ed113e2d19489259a43c3a35de9e00caacb1a6`, baseline
`b58816f64cfe1871dd97b50eaf9b0dac00699ded`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Six source/test files, 1242 insertions/72 deletions.
[Lifecycle contract](../lifecycle-contract.md), [public compatibility](../public-contract.md),
[error contract](../error-contract.md).

XpcClient adds is_usable, call_with_timeout,
call_recv_client_server_with_timeout, recv_with_timeout,
recv_client_server_with_timeout and recv_any_with_timeout. Calls share one
absolute cooperative Tokio deadline across send/flush/flow control, unrelated
replies and correlated receive. A single read shares selection/header/body
budget and provides an explicit idle policy. Legacy methods remain unbounded;
stream invocations have no new short request/lifetime timer. Zero returns
source-free TimedOut before polling/IDs/I/O, unrepresentable budgets return
InvalidInput, actual expiry retains Elapsed in existing XpcError::Io.get_ref.
Synchronous encoding/parsing/allocation cannot be preempted by these timers.

XpcConnection guards complete calls, sends and message reads. A successful
send or consumed 24-byte wrapper header commits ownership, including a header
buffered by an earlier H2 operation. Cancellation/failure after commitment or
lower H2 invalidation clears pending replies and their memory budgets, also
invalidates H2, and rejects later cached/wire operations. Original operation
errors remain owned/typed. The subsequent rejection uses the existing H2-to-XPC
adapter and Protocol family; no error enum variant was added. Empty idle waits
and unpolled futures stay usable. Complete ordinary messages can be reused and
their later service business errors remain outside the transport guard.

A complete stream-local failure before commitment retains other streams; an
abandoned committed call requires a fresh transport. is_usable is local
framing/request state, not peer liveness, availability of every stream, remote
ACK or safe retry. Message-counter exhaustion returns existing Tls without
wrap/panic/I/O; framing remains usable for reads. The transport remains owned
until client Drop, with no async shutdown/cleanup task.

Public raw stream invocations retain the request guard across yields. Dropping
the stream after send invalidates the client even between complete messages.
Dropping a next() polling future alone preserves the stream's held read. A
private service parser can explicitly complete the guard after a valid terminal
message: CoreDevice uses its unchanged finishStreaming grammar. Its empty ACK
filter, element order and status/error parsing remain. Finite Pasteboard PULL/
SET/RESOLVE uses a guarded uncorrelated send/nonempty-reply primitive, retaining
bodyless/empty-dictionary filtering and parsing business errors after completion.
This preserves normal request/terminal reuse while interrupted raw requests
require a new transport. Existing Pasteboard timeout wrappers remain.

Cryptex file sending now guards the announced preamble through host reads,
chunk writes, announced-size check and FIN. Cancellation or returned failure
after announcement invalidates ownership, including a zero-byte host-reader
wait between H2 operations. Original host IO survives. A successful FIN leaves
the receive half usable; it is a wire write, not an ACK/remote release. Existing
IDs/stream mapping, two-GiB limit, 64-KiB chunks and byte order remain. No new
file/reader budget, cancellation frame, detached reader or cleanup task is added.

| Owned host fixture | Evidence / boundary |
| --- | --- |
| Public budgets/unpolled/correlation | All five deadline methods reject zero, Duration::MAX rejects input, unpolled call/stream leaves IDs; first actual request is ID1, unrelated reply remains queued, stream-1 call and any-stream fresh read succeed |
| Sent call cancellation/expiry | Actual public request fully sent, reply withheld; original timer retained, complete late reply cannot satisfy later public entries, no second wire request and client Drop gives peer EOF |
| Shared send/reply deadline | An 8192-byte Data body blocks a 4096-byte transport; peer waits 125 ms before draining send and another 125 ms before reply. The single 200 ms budget expires; independent restarted receive timing would allow the reply |
| Blocked send/empty idle | Actual 4096-byte outgoing prefix then expiry invalidates; empty stream-3/any-stream waits stay usable, followed by a real stream-1 message |
| Previously buffered wrapper | An earlier H2 read on stream 1 buffers a complete 24-byte header on stream 3; later XPC body wait receives no new bytes, but expiry/caller cancellation invalidates and clears seeded pending replies/accounting |
| Partial raw frame/native EOF/bad wrapper | Actual four-byte prefix, closed peer or complete malformed wrapper; upper queues clear immediately and a previously cached reply cannot return after failure |
| Stream-local reset/counter boundary | Actual reset before message commitment preserves another stream's pending reply; private MAX counter injection rejects without wrap/I/O. This is not a short reachable runtime rollover |
| Raw stream cancellation | Actual request with flags0 and a fresh message, then Drop between yields invalidates owner; no new reader task |
| Explicit CoreDevice completion | Actual public bootstrap/request, bodyless ACK and existing finishStreaming envelope; stream ends normally and the same client sends again |
| Finite Pasteboard completion | Actual PULL requests, empty ACKs, a structured service error then success with fresh uncorrelated IDs; both requests complete on the same connection |
| Announced file ownership | Actual stream-5 HEADERS/preamble then pending owned duplex reader or original file-open IO; cancellation/failure invalidates without a detached FIN. Success sends content/FIN and permits a later response |
| External Rust consumer | Public bootstrap, zero/idle/successful and expired calls, all new API names/is_usable, original Elapsed and exhaustive four-variant match using only public imports |

Sixteen tests added: fifteen internal/one external; minimum tunnel runs eleven
internal/one external. XPC117, Pasteboard30 and CoreDevice10 regressions PASS.
New peer exchanges are owned joins with one-second bounds, without fixture
tasks. Pending queues and counter/window states use identified private seams;
they do not prove physical traffic, long-run load or device cleanup.

Preparation failed on two Vec-to-Bytes fixture values, a CoreDevice fixture
passing IndexMap instead of XpcValue, a missing CoreDevice feature gate in the
minimum tunnel test, and Clippy multiple bound locations. Test types/feature
conditions and generic bound placement were corrected without relaxing old
assertions or production ownership policy. Finite service caller inspection
also led to the explicit completion/atomic Pasteboard adapters before full.
No old-implementation negative control or throughput benchmark was run.

Canonical command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
First full session 34637 ended exit 0 with validation/full PASS. Frozen source
SHA256 `5742b3a91ffedae8a0978d52ee10049f9ae19a08669f384732e2e24d60237cfa`;
debug ios SHA256
`5f4c39cf383a877365c38286a312b17788dee61746f8cd0c47b23757f7b37dd5`.
Fourteen Rust groups: 2175 PASS, Python13 PASS, zero failures/two existing ignored;
CLI706/Core1316/public43. Fmt/check/strict workspace Clippy/MSRV, 66 features/
four groups, 7+7 doctests/three snippets, C ABI, Python3.12 wheel/sdist, 53 tooling/
118 help/38 raw/five host/15 CLI/three backup contracts PASS. Source frozen until
terminal; approximately nine GiB free, no clean/storage failure. Logs use
/tmp/ios-l01-xpc-: narrow*.log, minimum*.log, regression*.log, stack.log,
pasteboard.log, coredevice.log, clippy*.log, fmt*.log, source-audit.log,
source.patch, reference.log and full.log.

Source audit confirms all existing tests/assertions byte-identical, RSD
bootstrap/body reader/wire, pending budgets/queue helpers, CoreDevice grammar
and Pasteboard response parser unchanged. The moved file sender matches after
removing its ownership marker/borrow adjustment. H2 changes only invalidator
visibility; codec, root/error enums/classifier, frontends and dependencies stay.
Reference hashes/status in /tmp/ios-l01-xpc-reference.log match the prior H2
record, without reference edits/source copying or new protocol selectors.

Split send/recv, borrowed RSD bootstrap/handshake, other service/protocol totals,
file/reader deadlines and remote cleanup remain owner-specific L01 work.
L02–L04/Q04 and all 31 tasks/subtasks remain active. Twenty-two device rows are
NOT_RUN; Chromium and installed Python3.9/3.13 were not rerun. Capability maturity
is unchanged; no real devices/credentials/Apple actions, push/publish/history
rewrite or reference mutation occurred.
