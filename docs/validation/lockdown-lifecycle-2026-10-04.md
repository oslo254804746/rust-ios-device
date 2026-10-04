# Lockdown requests and session close

L01/class 4 host slice accepted 2026-10-04, source
`79f750e78b4550909c440fc4697b592101878935`, baseline
`0f4c31fc99440e7698a78716db028c59dbb81cdd`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Three source/test files, 769 insertions/38 deletions.
[Lifecycle contract](../lifecycle-contract.md), [public compatibility](../public-contract.md),
[error contract](../error-contract.md).

LockdownClient adds is_usable and set_request_timeout(Option<Duration>). None
keeps legacy unbounded requests. A configured absolute cooperative deadline
covers request encoding, send/flush and reply decoding. Zero rejects before
I/O/session changes; an unrepresentable request budget returns InvalidInput.
Actual expiry retains Elapsed in existing LockdownError::Io.get_ref. Synchronous
work cannot be preempted. Constructor/setup timing and totals across separately
completed requests or post-reply convenience parsing remain caller-owned.

Query/value/service/activation requests share a guard. Accepted read/write
bytes include buffered TLS halves. Cancellation/failure after progress, native
transport errors/EOF and frame decoding failures reject subsequent requests
before consuming cached or late replies. Original operation errors stay owned
and unchanged; later rejection is existing Protocol. No pending queue is added;
opaque halves/buffers stay owned until client Drop. Empty write waits/unpolled
futures and local serialization failure before I/O can preserve reuse.
Complete business replies retain their existing parser, legacy/preserving
classification and framing policy independently of their returned error.

StopSession now has an independent five-second write/flush budget, clamped by a
shorter configured request budget. It retains the private session identifier
until successful write, then marks closed. Later requests reject and repeated
stop returns Ok without I/O. Failed/abandoned close after accepted bytes or
transport failure requires a fresh transport; empty write expiry can retain
state for another attempt. A retained identifier indicates uncertainty, not a
confirmed active remote session. Successful stop remains a write, without a
reply/remote ACK. No automatic stop, async Drop or primary/cleanup aggregate is
added; manual callers own both results. There is no remote release, rollback,
safe retry or peer-liveness guarantee from local state.

Raw borrowed frame/session helpers and device-side fresh-session paths are
unchanged. Their surrounding owned operation drops the transport on cancellation;
raw borrowers still own reuse/total-budget policy. TLS setup/verifier, four-MiB
frame limit, request envelopes, response parsers and frontend shapes remain.
Helper futures are boxed before await state and remain directly owned, without
a task or background reader. Five original CLI 16-KiB stack tests pass.

| Owned host fixture | Evidence / boundary |
| --- | --- |
| Zero/overflow/unpolled/local input | Request/close zero emits no bytes and retains session state; Duration::MAX request rejects input. Unpolled query/close and genuine refused serialization preserve local state/source |
| Sent/fragmented/late replies | Complete query send then caller cancellation/expiry, or partial reply then cancellation; complete late reply cannot satisfy subsequent high-level requests. Peer sees no second request and EOF after client Drop |
| Shared budget | A 4096-byte value blocks a 512-byte duplex send. Peer drains at 125 ms, then sends the complete reply after another 125 ms. The single 200-ms budget expires; restarting receive timing would permit that reply |
| Empty/partial writes | A privately prefilled 64-byte direction makes the guarded write wait accept zero request bytes and preserve reuse. An actual accepted 64-byte request prefix makes expiry invalidate; exact prefix length is checked |
| Complete business/native failures | Complete MissingValue and InvalidService replies permit another successful query. Actual native EOF, malformed plist and oversized declared frame reject reuse and retain original failure family/source |
| Buffered length | Private BufReader is warmed with a complete four-byte length; query sends, consumes that cached length and waits for body. Cancellation rejects the late body without reading another cached reply |
| Session close | Exact StopSession envelope, successful closed state and local repeat; native closed write, partial caller cancellation, shortened expiry and empty expiry retain the identifier with the appropriate local state. Default request timeout None still yields actual five-second cleanup expiry |
| Actual TLS constructor | Real QueryType/StartSession/upgrade using owned synthetic material, then public query expiry and complete late reply; follow-up query/stop rejects. Server sees no extra application bytes and transport EOF without an async close task |
| External consumer | Public getter/setter signatures, eight named Send futures and exhaustive three-variant match compile. This fixture does not establish a device session or prove runtime method behavior |

Ten tests added: nine internal/one external, each passing with all features and
minimum default features. Lockdown73 regressions pass. New fixtures use owned
joins/pinned futures with one-second bounds, except two seconds for TLS and six
for observing the five-second close timer; no new fixture tasks. Synthetic TLS
uses no client authentication and does not establish Apple trust/device support.
No old-source negative control, throughput measurement or device acceptance.

First narrow passed eight tests with one unused-mut warning; it was removed,
then nine internal/one external passed. After the passing narrow pipeline,
StopSession caller cancellation was added and the total-budget fixture was
changed to send a complete late reply fitting the duplex capacity. Final narrow
and minimum each pass ten. A temporary audit's public-fixture slice offset was
corrected without product-source changes. Original assertions/input/parser
policies remain; six old client fixture initializers only gain default private
state. Source audit compares envelopes/parsers after mechanical send/receive
consolidation and keeps raw framing/TLS/session/device/root/error enums,
classifiers/frontends/dependencies/reference checkouts unchanged.

First full gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session36426 terminal0, validation/full PASS: 2192 Rust/13 Python, zero failures,
two existing ignored; CLI706/Core1332/public44. Workspace fmt/check/strict Clippy,
MSRV, 66 feature checks/four test groups, 7+7 doctests/three downstream snippets,
C ABI, 3.12 wheel/sdist/install, 53 tooling/118 actual-help/38 raw/five host/
15 CLI/three backup contracts pass. Chromium and installed 3.9/3.13 were not
rerun; 22 device rows remain NOT_RUN. Capability maturity stays unchanged.

Frozen source SHA256:
`a1a3730a55e5d55bc6ba3b86778a65a46b011a268c9a980806f304c98dee4a3f`.
Final ios debug binary SHA256:
`82571798d8abdfe8c6dada926616ad15ca760f4e2d02388ee468709ef378e755`.
Source stayed frozen through terminal full/minimum results. No clean/storage
failure; about 7.4 GiB remained. Logs `/tmp/ios-l01-lockdown-` are local;
this topic records durable outcomes. Narrow14654/32392, pipeline86481,
Stop-cancel14991, final-narrow10951 and final-minimum14987 all terminal0 are
separate from the full evidence.

L01/class4/all31 goal remains active. DeviceLink/plist/backup raw ownership,
borrowed raw-session callers, setup/service/multi-request totals, other cleanup/
long streams/manual-pairing exchanges, L02–L04/Q04 and platform/device conditions
remain. No device, credential, Apple-side operation, reference edit, push,
publication or history rewrite occurred.
