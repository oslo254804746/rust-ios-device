# Public compatibility rules

This policy governs Rust source, CLI arguments/results, C ABI and Python APIs.
It is an engineering compatibility contract, not a claim of device support.
The [capability catalog](capabilities.md) remains authoritative for maturity and
device/platform evidence. All current device workflow entries remain
experimental. The following profiles are **candidates for maintained compatible
interfaces**; their host fixtures preserve existing usage while A02, L01 and
the workflow tasks supply the remaining error, cancellation and device evidence.

| Profile | Candidate interface and current baseline | Boundary |
| --- | --- | --- |
| Rust high-level access | Root `ConnectOptions`, `DeviceInfo`, `DeviceEvent`, `ConnectedDevice`, `connect`, `list_devices`, `watch_devices`, `TunMode`, `TunnelInfo`/`Handle`/`Manager` | Async source signatures, public fields and existing root/module paths; no new cancellation/Send/layout guarantee |
| Rust metadata/codecs | Root `ServiceDescriptor`/`RsdHandshake`, `XpcMessage`/`XpcValue`, existing codec helpers and documented service feature aliases | Missing service differs from unadvertised features; codec/interface presence is separate from device protocol support |
| CLI selection/listing | `--udid`/`IOS_UDID`, `--no-json`, `-v`, `list` JSON and human/empty-list behavior | Debug requires explicit selection; command-specific scopes and output are not replaced by a global guessed schema |
| C existing ABI | All declarations/statuses/layouts in `ios_rs.h`, matching linked C smoke | Ownership and exact pointer/integer signatures; valid enum values only; physical connection/query acceptance remains pending |
| Python existing binding | `ios_rs.list_devices()`, `start_tunnel(udid, mode="userspace")`, `Tunnel` metadata/close/connect_info/asyncio_proxy and `AsyncioProxyPatch` context/restore methods | Installed import/signature/error boundary; application/files/log SDK and device workflows remain separate A03 tasks |

Private transport modules are `pub(crate)` and are not downstream imports. Use
documented root reexports and public `device`/`discovery`/`services` paths. Existing
published public paths stay available during R01–R03; moving internal files
cannot silently remove a root reexport or formerly public nested type. Service
features and aliases such as `icon_service`/`iconservice` and
`screen_capture`/`screencapture` remain additive and enable identical types.
Changes to default/group feature membership or an MSRV/dependency requirement
need an explicit compatibility and packaging review. Minimal feature builds
and feature-specific fixtures remain required.

## Rust changes

WebInspectorClient adds is_usable and set_operation_timeout(Option<Duration>),
retaining constructors/root paths and default None. Startup/timed events use
the shorter explicit/configured budget; an event includes automatic listing
writes. Zero now rejects before ready cache/IO as source-free Timeout and
unrepresentable effective budgets return Io InvalidInput. Startup overflow
previously returned Protocol; timed/discovery zero admission and startup's
effective-budget getter replace their former ready/remainder behavior. Shorter
explicit budgets can safely clamp MAX configuration. Actual expiry keeps Elapsed.

Partial/native/codec/routing failure invalidates and clears owned metadata/
pending state. Later event reads/wire writes reject Protocol, including cached events;
restore and setter None cannot revive state. Empty idle/send cancellation and
local XML failure before IO can preserve reuse. Startup guards a committed
report through CurrentState, while the legacy Disabled versus preserving-error
mapping stays. Discovery snapshots require empty usable idle; partial input or
configured total expiry now fails. Opaque stream stays until Drop and getters
are local admission, not retry authorization. Six variants/models/serde/wire
strings/16MiB/raw helpers and original consumer assertions stay. These pre-1.0
source/reuse/timing migrations need release review; no bypass/deprecation alias.
The client-only profile deferred correlated sessions/cache bounds/frontend
owners. See
[WebInspector client acceptance](validation/webinspector-client-lifecycle-2026-10-05.md).

The subsequent session profile keeps all Inspector/Automation public signatures
and six error variants. Correlated command and attach send/reply stages now
share one effective absolute total and invalidate a committed client on caller
cancellation, expiry or failure before the expected result. Another session or
clone cannot read a late reply on that client. Explicit matched business failures
retain the original response/error and local reuse policy; successful unrelated
events return in their original order. Empty blocked sends and unpolled calls
can preserve reuse. Discard sessions when reconnecting; their cached target/page
getters do not authorize recovery on another transport.

Pending plus skipped events cap at1024/16MiB compact JSON, including existing
pending state and restoration; rejection retains a private typed ResourceLimit
under Io. IDs admit the full existing u64 domain once, then reject exhaustion
before IO without wrap/reuse. Local limits are not inferred device limits or
RSS/peak-memory measurements. Zero is source-free Timeout, effective overflow
Io InvalidInput; native expiry retains Elapsed and the original effective total.
Finite scopes inherit through nested sends/startup/timed reads/automatic listing,
so MAX can be clamped without inner overflow; exiting restores independent
configuration. Old internal timing assertions remain test-only. Default None,
wire/models/selectors/business parsers/frontends and old consumers stay. These
pre-1.0 timing/reuse/resource migrations need release review. That profile
deferred application/page cache bounds; multi-command workflows and frontend
cleanup remain pending. See
[WebInspector session acceptance](validation/webinspector-session-lifecycle-2026-10-05.md).

The cache profile keeps all public signatures/models/serde, six error variants,
root paths and successful event/snapshot ordering. Incoming application/page
metadata now has fixed LOCAL counts and16MiB logical compact-JSON limits,
including scalar availability and map keys; raw lists and staged collections
are admitted before growth. open_pages_snapshot keeps returning Vec but the
corresponding flattened projection is separately admitted against16MiB before
caching, limiting repeated application copies across pages. Oversized complete
updates return existing Io with private typed ResourceLimit and invalidate the
receiving client; discard its sessions and construct a fresh client/session.

Duplicate page events keep their original order; cached duplicate IDs retain
the last value in first-ID order. Replacement/disconnect release count/byte
budget and owned removed payloads, with empty tables dropped. These fixed
pre-1.0 admission changes need release review. They are not device limits,
RSS/peak-decoder/recursion/object-reference/performance or retry guarantees;
the unchanged wire bound is checked before plist decoding, whereas model/cache
admission follows it. Frontend cleanup and further L01/Q04 remain. See
[WebInspector cache acceptance](validation/webinspector-cache-lifecycle-2026-10-05.md).

CLI application/page discovery preserves original terminal Timeout/native
Elapsed when core framing/routing invalidates the client. Its former follow-up
invalid-client Protocol is no longer substituted. Empty usable idle retries,
cached matches, page-key precedence, public grammar/output and all core APIs
remain. This error behavior migration needs pre-1.0 release review; setup/
whole-flow/pinned signal/join/primary cleanup remain separate work. See
[WebInspector discovery acceptance](validation/webinspector-discovery-lifecycle-2026-10-05.md).

The subsequent finite CLI profile covers opened-tabs, eval and launch. Default
selection moves inside a checked absolute total shared by connection, startup,
discovery, session commands and output. Explicit selection and first-device
ordering stay. The same pinned Ctrl+C future spans all stages; successful local
signals are typed Cancelled, while signal-registration IO remains IO. Zero now
rejects as source-free Timeout before discovery instead of using the former
100ms minimum; positive inputs keep that minimum. Invalid duration/clock input
rejects before discovery. Discovery uses the existing500ms idle cap inside the
total, so a smaller total can expire before quiescence.

Attached usable sessions receive a separate5s cleanup total. Completed output
can now be followed by cleanup failure and exit1; primary plus cleanup failures
retain both errors with the primary classification. Successful row/value/launch
schemas and human rendering stay, including the full eval response envelope.
Exactly one old root default-selection predicate assertion is migrated;
existing task/shell assertions and core APIs stay. These timing/exit changes
need pre-1.0 release review. Interactive shell and server owners remain separate.
See [WebInspector command acceptance](validation/webinspector-command-lifecycle-2026-10-05.md).

The interactive js-shell profile moves default selection into the setup owner.
One total covers selection, connection, startup, optional Safari launch,
discovery, attachment, navigation and session metadata. Idle input remains
unbounded and cancellable; each expression and its output receive a new total.
Zero rejects before stdin acquisition/discovery, replacing the former100ms
fallback; positive inputs keep that minimum. One pinned signal now makes active
Ctrl+C a deterministic normal stop after setup; setup cancellation still fails.

Continue-on-error applies to completed business responses. Native transport,
codec, deadline and input/output failures now stop instead of printing a request
error and attempting another command on the invalid client. A fully attached
usable session gets independent5s cleanup; cleanup or explicit stdin-restoration
failure can make an otherwise successful stop exit1. Session/result/error JSON,
human channels, page-key precedence, URL encoding, prompts/EOF and exit commands
stay. These timing/error/exit migrations need pre-1.0 release review.

Linux owns pipe/terminal stdin through readiness and restores modified flags on
explicit finish or best-effort Drop. Unpollable nonregular descriptors now fail
with their native registration error. Regular-file reads and synchronous output
remain cooperative. macOS/Windows retain the prior Tokio stdin path; cancellable
input/runtime shutdown there remains awaiting platform work and evidence.
No public core/binding API or wire change is added. See
[WebInspector shell acceptance](validation/webinspector-shell-lifecycle-2026-10-05.md).

CompanionProxyClient adds is_usable without changing existing constructors,
signatures or its four error variants. Shared client/listener/forwarding owners
reject further wire work after interrupted progress, a committed unanswered
request, native IO/EOF or malformed frame. A queued owner rechecks state after
the mutex; late replies are not consumed. Complete business failures retain
their former parsing/reuse policy. Empty send/lock cancellation and unpolled
futures can remain usable. The getter is local admission, not peer liveness or
retry authorization; opaque buffers live until all shared owners drop.

The configured ten-second request default remains. Request totals include mutex
wait/send/reply. Zero now rejects before any ready I/O as legacy Timeout(duration);
unrepresentable request/active-idle deadlines become Io InvalidInput rather than
panicking. Issued expiry retains native Elapsed/duration evidence. Listener next
poll cancellation retains its owned read; actual idle expiry stays terminal and
now releases that read/mutex immediately and invalidates shared reuse. Dropping
the whole listener invalidates too. Active zero/overflow idle rejection keeps
the listener intact; already finished reads keep their None outcome.

Forwarding stop via client, explicit handle and existing Drop task now has an
independent five-second cap, with shorter configured/explicit limits. A MAX stop
budget safely clamps to five seconds; this is a timing migration from the old
request/caller cap. Explicit completed success remains cached/idempotent for
zero/MAX later calls. Abandoned stop invalidates; complete business error can
retain reuse. The existing best-effort Drop task stays, skips stopped/invalid/
zero-configured state and rechecks after lock; it is not awaited cleanup or a
remote guarantee. Complete business-failed explicit stop can still receive its
old Drop attempt. That core profile introduces no new protocol key/selector,
task/queue, ACK, primary aggregate or CLI setup/signal owner. See
[Companion host acceptance](validation/companion-lifecycle-2026-10-04.md).

The later Companion CLI profile adds consuming ForwardingHandle::close and
close_with_timeout. Creating either future immediately disarms best-effort Drop,
including an unpolled or cancelled future; one awaited stop caps at5s (shorter
supplied budget, zero legacy Timeout, MAX safe). Borrowed stop/Drop remain.
Other shared owners keep the stream; no remote cleanup or safe retry is implied.

Companion CLI finite zero now rejects as InvalidInput before discovery, with
overflow also checked before default/explicit selection. Existing flags/help/
defaults, routes, success output/newlines and error v1 fields stay. Setup now
caps at30s and shares a pinned signal with all later work. Finite totals begin
before discovery; None still allows unbounded listener/forwarding wait. Setup
and finite request stop return typed cancellation; started Listen/Forward retain
graceful Ctrl+C. Issued expiry remains a failure but now retains actual Elapsed
instead of synthetic TimedOut. Forwarding cleanup is one independently awaited
5s attempt after returned failures or stop, even past the total; output/signal/
timer primary errors survive cleanup and whole caller Drop spawns no fallback.
These timing/error-source/ownership migrations require release review; there is
no bypass alias. See
[Companion CLI acceptance](validation/companion-cli-control-2026-10-05.md).

NotificationProxyClient adds is_usable and set_operation_timeout(Option<Duration>).
Default None retains unbounded sends/idle reads. Explicit waits use the shorter
configured budget; wait_for now includes ObserveNotification in one absolute
budget, with unrelated events unable to restart it. Zero returns existing Timeout
before I/O, including next_event's formerly issued zero timer; overflowing
ordinary budgets return Io InvalidInput. Actual expiry retains Elapsed.
Interrupted partial frames/native IO/EOF/typed plist decoding failures reject
all later operations before I/O; complete business-shape errors preserve reuse.
Empty idle expiry/cancellation retains completed subscriptions and late events.

ProxyDeath still returns the existing event/error, then closes local reuse.
Shutdown caps writes/flush at five seconds (shorter configured clamp, MAX safely
clamped); success closes reuse, including repeat shutdown. These subsequent
operations return existing Protocol. No ACK, automatic close or async Drop is
added; the transport remains owned until Drop. Existing five error variants,
both event enums, serde envelopes/typed decoder/incoming4MiB limit and frontend
production remain. Outgoing length now checks u32 conversion before writing;
that extreme allocation boundary is source-reviewed only. CLI totals/signal
ownership remains separate; bounded cleanup is covered below. See
[notification host acceptance and migration](validation/notification-lifecycle-2026-10-04.md).

Mobilebackup2Client adds is_usable and set_operation_timeout(Option<Duration>).
Default None keeps ordinary operations unbounded; DeviceUnback/Extract retain
their five-minute ceiling, shortened by a configured total budget. The owner
covers negotiation, command, host work, raw transfer and final cleanup. Zero or
overflow rejects before I/O; actual expiry retains Elapsed. Interrupted progress
and native failures reject all later operations before I/O; an empty idle wait
or local pre-I/O failure can remain usable. No late disconnect is sent after
operation expiry or during an abandoned raw transfer. Complete terminal business
validation still permits the bounded disconnect attempt. Expected peer closure
after that reply preserves the previous completion policy but prevents reuse.

Mobilebackup2Error keeps its four variants and adds cleanup_failure(), borrowing
an opaque SessionCleanupFailure with operation_error()/cleanup_error(). Dual
failure uses existing Io, retains both original objects and follows the primary
in its source chain/category. This replaces the former cleanup-timeout-first
classification for affected dual failures. An outer expiry while cleanup waits
also preserves an earlier operation error. The older device_operation_timeout()
getter remains, with bounded source lookup. Successful disconnect closes local
reuse; no asynchronous Drop, remote ACK, rollback or safe retry is implied.
Cancelled blocking awaiters may leave workers running. Wire, parsers, transfer
limits, frontend schemas and dependencies remain. See
[MobileBackup2 host acceptance and migration](validation/backup-lifecycle-2026-10-04.md).

DeviceLinkClient adds is_usable, set_operation_timeout(Option<Duration>) and
call_process_message. Default None keeps unbounded message/idle timing. One
absolute budget covers a complete version exchange or process send/reply;
separately completed send/receive calls keep separate ownership. Zero/overflow
reject before I/O, actual expiry retains Elapsed in existing Io.get_ref.
Interrupted operations after accepted bytes and transport/frame-decode failures
reject reuse; complete process-envelope validation keeps its existing policy.
Disconnect now caps write/flush at five seconds, clamped by a shorter configured
budget. Success closes local reuse and subsequent disconnect also returns
existing Protocol; there is no remote ACK or asynchronous Drop. Native EOF in
MobileBackup2 now retains the original error directly and prevents a cleanup
write on that invalid DeviceLink connection. Raw stream_mut/into_inner signatures
stay available with caller-owned framing/cancellation; they do not repair an
invalid transport. No wire, codec limit, error enum, frontend or dependency
change. See [DeviceLink migration and acceptance](validation/device-link-lifecycle-2026-10-04.md).

LockdownClient adds is_usable and set_request_timeout(Option<Duration>). Default
None keeps unbounded request timing; a configured budget shares send/flush/reply
decoding. Zero/overflow reject before I/O; actual expiry retains Elapsed.
Interrupted requests with accepted bytes and transport/frame-decode failures
reject reuse; complete business replies keep the existing legacy/preserving
error policy and framing. Local serialization failure before I/O remains usable.
StopSession now has a separate five-second write budget, clamped by a shorter
configured request budget. Successful write marks the client closed; subsequent
requests return existing Protocol, while repeated stop returns Ok without I/O.
Failed/abandoned stop retains uncertain session ownership rather than reporting
an idempotent close. No remote ACK is awaited. Existing constructor, raw borrowed
APIs, error variants, wire and frontend shapes stay; no automatic stop or new
setup budget is installed. See
[Lockdown migration and host acceptance](validation/lockdown-lifecycle-2026-10-04.md).

Borrowed internal RSD initialization, bootstrap queuing and wrapper/handshake
reads now reject reuse after cancellation or failure following a complete send
or consumed header/message. Empty idle waits and successful completions preserve
local framing; a stream-local reset before logical commitment still permits
other streams. Existing caller budgets, owned fresh-transport fallback order,
wire/ACK acceptance and error sources remain. There are no new root exports,
public signatures, error variants or frontend options. Separately completed
bootstrap/read helpers remain caller-owned stages. See
[RSD migration and host acceptance](validation/rsd-lifecycle-2026-10-04.md).

XpcClient adds is_usable, call_with_timeout,
call_recv_client_server_with_timeout, recv_with_timeout,
recv_client_server_with_timeout and recv_any_with_timeout. Calls share one
send/flow-control/reply budget; single reads have explicit idle budgets.
Legacy entries retain unbounded timing. Interrupted committed calls/messages,
raw stream invocation Drop and announced file cancellation now require a
fresh transport and reject cached replies. Empty idle reads remain reusable;
complete finite Pasteboard replies/business errors and CoreDevice
finishStreaming preserve normal reuse. Existing error enums/wire/frontends stay.
See [XPC migration and host acceptance](validation/xpc-lifecycle-2026-10-04.md).

H2 frame operations now reject reuse after transport failure or cancellation
with reported bytes. Empty idle cancellation and fully processed stream-local
reset/EOF/GOAWAY retain their previous behavior. This affects existing public
XpcClient operations when their lower-level frame operation is interrupted;
the H2 type/configuration remains crate-internal, with no new root export,
CLI option or default deadline. The later XPC profile above supplies message,
sent-call and pending-reply ownership beyond this H2-only slice. See
[H2 host acceptance and migration](validation/h2-lifecycle-2026-10-04.md).

AfcClient adds is_usable and set_request_timeout(Option<Duration>). The optional
budget covers each complete send/reply request; constructor default None keeps
legacy unbounded requests, without a new whole-file/setup/reader budget. All
requests reject after IO/codec failure or cancellation with bytes; complete
ordinary Status errors still permit reuse. Automatic file reads/writes now close
on returned failure with a separate five-second budget and invalidate ownership
on failed close or cancellation after open. Dual failure can return existing Io
instead of primary Status, while retaining both originals and the primary's
category; single failures retain their original variants. Drop cannot await
close. Existing error enums/wire/limits/C/Python interfaces stay. See
[AFC migration and host acceptance](validation/afc-lifecycle-2026-10-04.md).

The instruments feature adds iter_frame_commit_timestamps, an allocation-free
iterator borrowing the input chunk and copying Mach timebase values. Existing
parse_frame_commit_timestamps still returns Vec with the same conversion/filter
policy. FPS CLI keeps flags, sample calculation, NDJSON fields and human rows;
zero windows now fail as InvalidInput, timestamp overflow as CLI resource_limit,
and failed cleanup after count/stop is reported instead of ignored. Setup/stop
budgets are 30/five seconds and the owned capture future has no background
reader. Whole-future cancellation drops the transport; a partial DTX read may
prevent a successful stop. Existing errors/C/Python interfaces remain. See
[migration and host acceptance](validation/fps-lifecycle-2026-10-04.md).

DTX adds request_channel_with_timeout, method_call_with_timeout,
recv_with_timeout and is_usable. Existing unbounded methods retain their timing
profile; errors or cancellation after wire progress now reject further I/O and
clear decoder state. Empty idle waits remain reusable. CoreProfile/accessibility
partial-frame timeout now fails rather than returning an apparent success/None;
CPU best-effort stop after EOF no longer writes on the invalid connection.
Existing exhaustive errors/wire/success results remain. See the [L01 lifecycle profile](lifecycle-contract.md)
and [host gate](validation/dtx-lifecycle-2026-10-04.md).

CLI owner-issued timeout guards use typed input/timeout decisions. A private CLI
state marker maps to precondition only in the final machine envelope; it does
not request confirmation, and remains Unknown to the general Core classifier.
No public API/enum/binding/schema changed. Partial accessibility traversal and
the original source-scan budget remain. See [owner-guard migration](validation/cli-owner-guards-2026-10-04.md).

The preserving Lockdown query/service APIs add an opaque LockdownResponseError
with private fields and bounded typed lookup. Existing exhaustive LockdownError
and CoreError remain. Legacy methods keep Protocol/String and shape policy;
opt-in methods expose exact device Error codes and explicitly reject a malformed
non-string GetValue Error. Backup/debugserver use typed observations for defaults
and fallback, preserving native failures instead of matching error text.
Returned source/Display/class/correlation migrate; existing wire/results and CLI
exit/v1 shapes stay. Explicit raw code access follows the private-data boundary.
See [response-code evidence and migration](validation/lockdown-response-codes-2026-10-04.md).

Existing public struct literals are legal downstream code. Adding a required
field, making any current public field private, changing its type, or adding
`#[non_exhaustive]` to an already constructible struct can break that code.
Existing public exhaustive enums can be matched without a wildcard; adding a
variant or retrofitting `non_exhaustive` can break their consumers. Workspace
compilation alone does not evaluate these effects. Do not apply those changes
to `ConnectOptions`, `DeviceInfo`, `CoreError`, `DeviceEvent`, `TunMode` or the
metadata types as a routine cleanup. A02 should prefer additive classification
methods/types which preserve current error variants and original error chains.

New extensible options/state types should use private fields with constructors
or builders from their first release. New enums which are designed to grow can
start non-exhaustive; callers then have the wildcard contract from the start.
Document defaults, invariants, feature availability and ownership. Adding an
inherent method is usually additive, but consider trait-method name collisions,
new generic bounds, auto traits, lifetime/Send behavior and inference effects.
Review downstream fixtures before classifying a change as compatible.

Rust object memory layout is not a public ABI unless explicitly specified.
Serde result shapes are a separate contract: adding fields can break strict
deserializers or consumers performing exact comparisons. Do not conflate a
source-compatible change with a machine-output-compatible one. Advanced
protocol APIs remain experimental; changed private-protocol behavior needs
protocol evidence and scoped fixtures, not just a method signature.

The additive Rust `ErrorClass`/`classify_error` and RSD feature-evidence APIs
retain existing exhaustive CoreError matches, descriptor fields and permissive
checks. Their [contract](error-contract.md) keeps typed category, successful
directory evidence and unknown probe state separate; it does not certify new
machine fields in the existing CLI/HTTP/C/Python interfaces.

`ConnectedDevice::initial_rsd_probe()` additively borrows an immutable historical
observation. New private-field `InitialRsdProbe` and non-exhaustive state/skip
enums are exported at root and `device`; old public fields, error variants and
optional directory getters remain. The probe owner retains failures instead of
silently treating them as unattempted; private proxy dials now retain typed IO.
Later on-demand retries do not update this initial snapshot or legacy cache.
See the [error contract](error-contract.md) for exact states, sensitive explicit
source access and remaining inner-adapter/lifecycle boundaries.

On-demand RSD service failures now retain the final probe CoreError/source after
the existing retry schedule; a successful-directory lookup miss uses the existing
XpcError::ServiceNotFound variant. This changes returned failure variants from
opaque Unsupported to their native category without extending the exhaustive
public error enums. Migrate by inspecting class/source; local missing-feature
prerequisites remain Unsupported. See the [error contract](error-contract.md)
for initial/latest observation, discovery and screenshot fallback boundaries.

H2 failures in RSD/RemoteXPC setup, bootstrap and message/transfer IO now reach
the existing XpcError::Io variant with their original H2/transport source and
fixed context. Native IO kinds retain their class; typed H2 protocol failures
use InvalidData. This changes the former Tls return variant, Display prefix and
diagnostic correlation without adding public variants/signatures. Migrate by
inspecting class/source rather than Tls text; protocol/input failures whose
sources were already strings retain their existing representation. The
[error contract](error-contract.md) records local host/drop evidence and remaining
partial-operation/lifecycle limits.

Syslog keeps its NDJSON fields and human bytes, but relay read/EOF failures now
exit 1 through the final safe error path instead of returning success. Prior
complete records remain available; incomplete relay messages are not emitted,
and an output failure may leave a partial final JSON record. Record write/flush
and stderr progress failures are checked. Matching-count and chosen capture-time
stops remain successful. See the [error contract](error-contract.md) for native
classes, partial-result migration and local drop versus device cleanup evidence.

Application mutations now honor global JSON mode with new compact version-1
tagged result records. `pkill` emits ordered NDJSON progress and a final complete
summary only after all matching signal calls succeed. Prior records survive
failure; output failure prevents the next call. Human mode retains its old
bytes and other app JSON queries retain their existing shapes. Migrate scripts
that parsed the old human statuses in default JSON mode by selecting `--no-json`
or the new version/tag fields. See the [error contract](error-contract.md) for
exact fields, signal-request counts, output/cleanup uncertainty and host scope.
Named typed appservice misses retain existing fallback behavior; native failure
sources now survive DTX/deadline/prerequisite contexts without new enum variants.

Streaming install adds `apps::install_ipa_with_checked_progress` and
`CheckedProgressCallback`, retaining the old function/void callback and exhaustive
error enum. Observer IO failure preserves its source and returns before reading
later replies. CLI startup/progress stderr now checks write/flush and returns
through the safe failure path; previous progress bytes and success JSON are
preserved. Migrate fallible Rust observers to the additive entry. A partial
stderr stream or absent success result can follow payload transfer; do not retry
or reuse a failed connection from the error alone. The [error contract](error-contract.md)
records native classes, borrowed-stream lifetime and host versus device scope.

Both streaming install entries now return ZIP decoder failures through the
existing `ZipConduitError::Io`, retaining typed causes instead of flattening
them into `Zip(String)`. The exhaustive enum and callback signatures stay
available. Migrate decoder-specific `Zip` matches to typed sources or
`classify_error`; known local package errors remain invalid input, while native
read errors use their kind (CRC InvalidData becomes protocol). Fixed phase
context replaces entry-name context. See the [error contract](error-contract.md)
for the complete returned-variant and diagnostic migration.

Remote-symbols H2/XPC adapters now return `FetchSymbolsError::Io` with retained
typed causes instead of `Protocol(String)`; genuine protocol failures remain
protocol and native IO uses its kind. Modern `ConnectedDevice::activate` service
failures similarly use existing `CoreError::Io` instead of `Other(String)`.
Pasteboard notification snapshot connection failures use existing
`PasteboardError::Xpc` instead of a protocol string. Exhaustive enums and public
signatures stay available; migrate the affected variant matches to typed
causes or `classify_error`. CLI symbols/activation/pasteboard retain these
sources, with fixed contexts and unchanged success output/failure envelope
fields. Activation's existing deadline and observed Ctrl-C failure now classify
as timeout/cancelled; effects stay unknown. See the [error contract](error-contract.md)
for scope, partial-file behavior and host versus device evidence.

`ConnectedDevice::deactivate` now retains both failed attempts through existing
`CoreError::Io`. With `mobileactivation`, additive root/device
`DeactivationFallbackError` has private fields and borrowed cause/category
getters. Obtain it from the IO custom payload using `get_ref().downcast_ref()`;
its standard source is the final lockdown failure. The final category, including
unknown, controls classification. Earlier failure remains separately observable.
This replaces a combined Other string and also preserves the first failure
when lockdown connection establishment fails. Primary success skips fallback;
fallback success remains successful. No new retry or device cleanup guarantee.

Activation HTTP native builder/send/body errors and endpoint rejections now use
existing `MobileActivationError::Io` instead of `Http(String)`. Inspect typed
sources/classification rather than old Http matches or text; endpoint validation
becomes invalid input, native timeouts become timeout, and other native HTTP
causes use the existing typed-chain/IO fallback policy. Status/body-budget
rejections still use Http. Client Debug hashes URLs; explicit original sources
can contain private URLs. Public signatures/enums, HTTPS verification, opt-in,
redirect policy, budgets and success response shapes remain. The
[error contract](error-contract.md) records host fixture and partial-effect scope.

WebInspector's additive `start_preserving_errors` retains startup IO/native
Elapsed under its existing Io variant;
the existing `start` keeps its legacy phase-specific Disabled mapping and its
exhaustive error enum. CLI/CDP/WebDriver startup uses the new method, changing
safe failure diagnostic categories when the original source is available,
with existing status/exit/body shapes and success output retained. The
[error contract](error-contract.md) records phase budgets and migration.

WebInspectorError and CompanionError also add `timeout_duration()` for their
own issued deadline or legacy Timeout value. Native WebInspector/Companion/
Notification timers now return existing Io with their original Elapsed instead
of Timeout; received parser/conversion faults keep protocol and local frame
encoding conversion becomes invalid_input. Existing public families remain
6/4/5 exhaustive variants. Migrate affected variant/Display/immediate-payload
matches to typed source/category inspection; the bounded getter's None can still
accompany an IO timeout. It reports the former phase budget, not current time left
or cleanup/retry safety. The [discovery evidence](validation/discovery-native-sources-2026-10-03.md)
records preserved quiescence, legacy source loss and partial-operation limits.

`MessageFilterOptions::compile_preserving_errors` adds typed OS Trace filter
compilation without changing the legacy compile method's String contract.
Core/CLI callers use the new method; invalid regex/count/length and CLI PID errors
now supply invalid_input with native sources where a native failure exists.
CrashReportError's five/OsTraceError's four variants stay exhaustive, while native
timers/parser/encoding/archive errors move to existing Io and preserve original
causes. Migrate affected Timeout/Protocol/text/immediate-payload matches to typed
class/source inspection. Crash watch retains its named parse retries, and
created guards/EOF/wire/success/budgets/staging/cleanup stay. The
[diagnostic evidence](validation/diagnostic-native-sources-2026-10-03.md) records
legacy string source loss, IO kind migration and host/device boundaries.

Cryptex/MobileActivation/InstallCoordination/Pasteboard retain 21 native causes
under existing Io or Xpc(Io), keeping their 11/5/3/7 public variants/fields
exhaustive. Additive `timeout_phase()`, `timeout_duration()` on activation and
InstallCoordination, and `timeout_seconds()` on Pasteboard preserve the former
Timeout payloads, including truncated whole seconds. Bounded typed getters
recognize their private issuer or legacy Timeout, returning None for generic
IO timeout or unissued Elapsed. These values do not imply retry or cleanup.
Native timer/HTTP/parser/cache/connect variants, Display and immediate source
payloads can change; use typed classification/source traversal. Created deadline
markers and existing budgets/wire/state/fallback remain. The full-width promise
size comparison also rejects values that could bypass a 32-bit usize limit.
The [service deadline evidence](validation/service-deadline-native-sources-2026-10-03.md)
records getter scope, native class changes, exact API families and host limits.

Mobilebackup2Error retains its four exhaustive variants and existing facade
signatures/fields. Unback/Extract native timers now return Io TimedOut instead
of Protocol text. The additive `device_operation_timeout()` borrows issued
`DeviceOperationTimeout`, with private fields and operation/phase/budget getters.
Its new operation/phase enums start non-exhaustive. The getter is bounded to
32 typed sources, does not format errors and does not attribute generic IO or
foreign timers to this service. `operation_error()` preserves a prior failure
when disconnect also times out; the final source/class remain Elapsed/timeout.
Received path UTF-8/count conversions move to Io InvalidData while retaining
protocol classification. Migrate variant/Display/immediate-payload matches to
class/source and typed evidence. Public requests, five-minute budget, successful
output and ordinary best-effort disconnect stay; successful content is still
lost on disconnect timeout. No new reuse/rollback/remote cleanup promise. The
[backup deadline evidence](validation/backup-deadline-native-sources-2026-10-03.md)
distinguishes internal short-budget behavior from external interface coverage.

RestoreError remains exhaustive with three unchanged variants; existing client,
IPSW/TSS/result fields, signatures and root paths stay. Twenty-five H2/XPC/IPSW/
TSS adapters now return Io with original objects instead of Protocol strings.
IO kinds/categories survive; local package/serialization errors become input,
received codecs stay protocol, HTTP expiry becomes timeout and other HTTP/XPC
categories remain. Migrate variant/text/immediate payload matches to typed
class and bounded source inspection. That source-retention change adds no public
method, enum field, dependency or feature. Wire, 30/120s TSS budgets, body/archive/firmware bounds,
identity/nonce/fallback rules and successful output stay. Legacy batch fallback
still discards per-attempt errors behind failed updater names. The
[restore evidence](validation/restore-native-sources-2026-10-03.md) records native
versus injected OS faults, encoding/size review and physical-operation limits.

The additive `restore::tss::prefetch_tss_batch_preserving_errors` returns an
opaque `TssBatchOutcome`. `result()` borrows the existing `TssBatchResult`;
`failures()` observes independent attempts in encounter order. New phase/kind
enums start non-exhaustive; retain a future-variant branch. `rider_index()` refers
to the supplied input, preserving duplicate-name distinctions. Original errors
and their bounded typed classes are optional; a missing, empty or non-data
ticket has neither a fabricated error nor a guessed class. Recovered attempts
remain visible even when all final tickets succeed. Use `failed_updaters` for
the existing final failure semantics. `into_parts()` retains owned observations,
`into_error()` takes a native error, and `into_result()` deliberately drops them.

Existing constructible rider/ticket/result fields, exhaustive RestoreError,
legacy signatures, request/fallback order, UUID merging and exact nonce checks
stay. The legacy path allocates no observation storage and releases ignored
signer errors before the next call. The new path holds those errors until
consumed/dropped; their payload sizes are supplied by the caller's signer.
Debug exposes metadata/counts only, while explicit error getters follow source
privacy rules. No CLI/HTTP/C/Python output change or new cleanup/retry promise.
The [batch observation evidence](validation/tss-batch-observations-2026-10-03.md)
records host, minimum-feature and downstream-consumer acceptance.

AppServiceError remains exhaustive with Xpc/Protocol; process-identifier and
fixed empty-plist encoding errors now retain native causes under Xpc(Io).
Testmanager's DtxError keeps its three variants, ZipConduitError five and
SimLocationError two. Their affected integer/XML/time failures use existing Io
InvalidInput. PID/ZIP categories change from protocol to invalid_input; GPX
classification stays invalid_input while its returned variant changes from
GpxParse. Existing root paths, constructors, method signatures and published
route/process fields stay. Migrate returned-variant/text/immediate-source matches
to typed class and bounded native sources; safe diagnostic IDs can change.

No new public method/type/enum field, feature or dependency. The fixed app plist
helper is private and preserves the original empty-dictionary bytes. Public
writer use remains the original Vec; injected writer failure is host boundary
evidence, not a new caller-controlled encoder. Request/feature/control admission,
ZIP/GPX wire, validation order, GPX route timing and
success stay. Existing unchecked location string length casts, parser limits
and lifecycle/retry semantics are separate work. The
[input source evidence](validation/service-input-native-sources-2026-10-03.md)
records 9 internal/2 external tests and public zero-write versus private scalar/
header/helper coverage.

CLI's native service/parser/file/Elapsed adapters retain their original objects
instead of formatted text. Owning input and received-data wrappers classify
Input/Protocol without changing Core/root/FFI/Python APIs or schemas. Unknown
remains valid for opaque native causes. Legacy input Strings are retained as
opaque data with no invented native source. Existing OperationCancelled now
identifies three CLI cancellation paths; HID returns signal-registration IO
failure separately. Request order, configured budgets, buffers and successful
output stay. Returned category/Display/source/correlation may migrate; final
exit 1 and unknown effects do not authorize retry or rollback. Source/privacy
and exact host boundaries are in
[CLI native acceptance](validation/cli-native-sources-2026-10-04.md).

CLI stream adapters also retain Pcap/FPS native errors and XCTest/notification/
CrashReport Elapsed. Notification ProxyDeath is typed disconnected; issued
Companion/OS Trace deadlines are source-free timeout. Failed signal waits in
OS Trace/crash/power hold propagate IO rather than completing successfully.
Normal operator stops retain success. JSON WebInspector shell omits interactive
prompts/EOF presentation bytes; existing result records stay. Categories,
Display, source traversal and safe correlation can migrate. Prior valid records
or file/device effects can precede exit 1; no rollback/retry/cleanup certificate.
The [stream inventory and host evidence](validation/cli-stream-contract-2026-10-04.md)
records compact/pretty/aggregate formats and the retained stop policies.

CLI status writes retain native I/O errors instead of panicking. Ordinary
status write failures return through the existing exit-1 boundary; six diagnostic
sites keep an already-failed operation primary. A failed status can follow output
or effects. Sysmontap CPU read errors now return failure after the existing stop
attempt, preserving prior rows; JSON rejects non-finite CPU load before that row.
Successful finite JSON/human records retain their formatting. Stop errors remain
best effort, with no new cleanup/reuse guarantee. Machine error schema and
stdout/stderr roles stay; final reporting can itself fail on closed stderr.
Unix directory path conversions retain original NulError with existing input
classification and fixed safe context. No public SDK shape changes. See
[diagnostic acceptance](validation/cli-diagnostic-sources-2026-10-04.md).

Discovery/connection native causes now survive fifteen adapters under existing
CoreError/TunnelError::Io or IO. Root error families/signatures/fields/exports,
feature/dependency lists, classifier and binding fields remain. The handshake
helpers stay behind the existing private tunnel module; no new SDK export.
Opaque mDNS backend objects retain Unknown; local MTU validation is InvalidInput
and received-data wrappers retain Protocol. Real handshake Elapsed migrates
from Protocol to Timeout; RSD check-in/lockdown dial retain Timeout with original
Elapsed and configured Duration. The minimum-MTU guard has no fabricated source.
Native objects and safe private context replace text flattening, so returned
variant, Display/immediate-source and diagnostic correlation can change. Use
typed classes and bounded source traversal. Wire, limits, shared budgets,
task/stream/daemon policy and success stay; no new retry/cleanup/reuse promise.
See [connection acceptance](validation/connection-native-sources-2026-10-04.md).

The eight guarded AFC/DTX/FileService/symbol adapters use existing Io variants
to retain original cast/integer/slice sources, with Protocol classification
through InvalidData and safe private context. Their public exhaustive families,
fields/signatures/root exports/core feature list/classifier and wire stay.
Returned variant/Display/immediate-source and diagnostic correlation can change;
migrate string/Protocol-variant assumptions to typed class and bounded sources.
The AFC cast owns its original bytes through zerocopy map_src only on failure.

Zerocopy's existing std dependency feature is enabled for the native StdError
implementation on MSRV; version/lockfile stay. Six native failure branches are
guarded on all hosts; only entire_len and symbols count need narrower-host
runtime evidence. Current 64-bit consumer tests preserve frame rejection and
local duplex EOF; private native fixtures do not certify those production
branches or remote cleanup/reuse. No new public method/type/field, deadline,
retry or allocation policy. See the
[guarded source acceptance](validation/guarded-native-sources-2026-10-03.md).

The additive `dproxy::StreamDecoder::push_preserving_errors` returns opaque
`DecodeOutcome`. Events, independent original errors and the optional fatal
error can coexist. Borrow them through `events`/`failures`/`fatal_error`, or
retain ownership through `into_parts` and a failure's `into_error`. New phase
and kind enums start non-exhaustive; fields are private. Failure event indices
refer to this push, with None for fatal guards, and XPC observations can carry
stream IDs. Source-free decisions have no invented source/class. Debug exposes
metadata/counts, not explicit payload/error text. `input_ignored` distinguishes
the sticky DTX failure policy from ordinary incomplete decoding.

Original ProxyEvent fields/serialization, four exhaustive DproxyError variants,
Direction/ProxyProtocol, root paths/features/dependencies and classifier stay.
Old `push` deliberately drops observations and preceding events on fatal error;
the new outcome's `into_result` has the same policy. Successful JSON, parser
limits, guard text and buffer/stream/error recovery behavior stay. Timestamps
remain wall-clock values. The empty DATA missing-stream fatal behavior is
preserved, not a new protocol guarantee. Bindings do not consume this API,
and there is no new resource
cleanup/retry/reuse certification. See
[DProxy decoder acceptance](validation/dproxy-decoder-observations-2026-10-03.md)
for original-source versus local-guard coverage and minimum-feature consumers.

The additive recorder/proxy `*_with_observer` methods accept FnMut callbacks
receiving owned DecodeFailure values; the async proxy requires Send and shares
one serialized callback for both directions. Observations precede event writes
and fatal return, but follow raw writes. Indices remain per-decoder-push, and
may refer to events skipped by fatal decoding or subsequent write failure.
Primary recording/forwarding errors still return through the original Result;
delivered decode failures remain separate owners. No new public type/enum variant,
constructor/field, feature, dependency, binding or machine JSON field.

Existing methods retain unobserved mode, with no observation allocation. CLI
selects observer mode, reporting metadata-only DEBUG diagnostics at `-vv`.
Callbacks run synchronously under the recorder mutex, must bound their own
retention, and must not block; their panics are not caught. Successful capture
bytes/file layout, summary/status text and forwarding/flush/shutdown/try_join
order remain. Core event and two CLI status writes now return native stderr
IO instead of panic, through existing error variants/failure formatting.
Partial capture writes and remote cleanup/reuse remain unproven. See
[recorder/CLI acceptance](validation/dproxy-recording-observations-2026-10-03.md).

ImageMounterError's six variants/fields and public downloader/TSS signatures stay
exhaustive. Thirty-four native cache/download/ZIP/join/TSS failures move from
Download/Tss strings to existing Io, preserving original sources and IO kinds.
Native HTTP, parser, local serialization and host cancellation categories can
change from opaque/service_failure to typed timeout/protocol/invalid_input/
cancelled; panic stays unknown. Existing guard outcomes and successful requests,
proxy/budget/cache/partial-file behavior stay. Migrate variant/text/immediate
payload matches to typed class and source traversal, respecting native source
privacy. The [DDI/TSS evidence](validation/image-assets-native-sources-2026-10-03.md)
records exact callback scope and owned versus injected/device validation.

The additive private-fields `OperationCancelled` marker records caller-observed
local cancellation without changing CoreError variants. Plain EINTR stays IO.
WebInspector launch's successful Ctrl+C branch retains its context and exit
while supplying this typed source; JSON failure class becomes `cancelled`, with
effects still unknown. Signal failure remains IO and cleanup remains unconfirmed.

Connection discovery/pairing native failures now retain JoinError, PSK/Pairing
errors, credential parsers, Base64/AEAD, address and slice parsers through the
existing CoreError::Io variant and private fixed context. Categories preserve
the original source, including unknown for opaque crypto/backend/panic causes;
local credential syntax is invalid input and malformed received encodings/key
length/address or failed response authentication is protocol. Three dial-budget
adapters also retain Elapsed, with unchanged timeout values. Migrate affected
Other/Protocol matches and Display parsing to class/source inspection; explicit
native sources can still contain sensitive data. Enums, signatures, wire and
trust/routing/retry policies stay unchanged. The existing backend-unsupported
PSK case now supplies its original OpenSSL source.

Inner pairing seed, HKDF, cipher and device-info encoding failures now retain
native objects under the existing internal PairingError::Io. Internal
Crypto/Protocol matches and Display parsing migrate to class/source inspection:
local length faults become invalid_input, received authentication failure protocol,
OPACK keeps its typed class and opaque encryption stays unknown. Nested
credential loaders keep the inner source as well. PairingError remains
exhaustive with the same three variants in the crate-private lockdown module,
available only with tunnel; no external API or default-profile type is added.
External callers retain CoreError and can inspect longer credential-loader
source chains through existing APIs; affected Display/correlation can change.
The public manual-pairing transport now retains all four inner error objects,
including the native M6 authentication chain. Sixteen outer source adapters
and the dial deadline retain XPC/H2/crypto/codec/Elapsed objects under existing
PairingTransportError::Io. Migrate affected Xpc/Rsd/Crypto matches and Display
parsing to class/source: native IO kinds, named service-missing/closed states,
received protocol faults and opaque unknown remain distinct. All nine public
variants, the root pair_new_device future/signature and credential fields stay.
Short internal header fields now return typed protocol errors rather than panic.
Body-limit String guards retain their existing variant/diagnostic, and earlier
legacy string sources cannot be reconstructed. Full pairing/device, reuse and
cleanup acceptance remains pending despite owned lower-stage source tests.

The workspace enables the existing chacha20poly1305 and hkdf dependencies' std
features to retain their native AEAD and InvalidLength objects as standard
sources. This adds Error implementations in an already std/Tokio workspace;
it changes no dependency version, lockfile, service feature names/defaults/groups
or cryptographic parameters, successful payloads, signing or trust behavior.
No no_std support was previously offered. Minimum-feature/MSRV/full gates and
the existing downstream consumer cover this dependency-feature migration.
The [error contract](error-contract.md) records category snapshots, changed
contexts, remaining legacy string outcomes and host versus device scope.

## CLI and JSON

Notification commands now attempt Shutdown after a returned operation/output
failure when the core client remains usable. Partial framing/native failure and
ProxyDeath skip that unsafe write; the command drops the transport. Unexpected
cleanup failure after success now fails the command, even if success output is
already written. Previously error branches skipped cleanup and final success
ignored its result. Dual failure owns both original errors in a private CLI
aggregate; source/category follows the primary, and automatic Display/Debug
never formats either cause. Existing JSON fields, help, parameters, event
selection and success bytes remain. Whole-future cancellation drops owned
transport without asynchronous cleanup. See
[cleanup host acceptance](validation/notification-cli-cleanup-2026-10-04.md).

The notification CLI control migration rejects finite --timeout 0 and
unrepresentable deadlines as invalid_input (exit1, phase notify) before explicit
or default device selection. Arguments/help/defaults and v1 JSON fields remain.
Default discovery, device connect and NotificationProxy service connect share
one 30-second setup budget. First-device selection and empty-default failure
remain; device selection for other commands is unchanged. No pairing or retry
is introduced. Setup Ctrl+C now returns typed cancellation.

After setup, Post and each full subscription stage have a 30-second send cap.
Roundtrip/Wait/Springboard and Stream --timeout now share the user budget from
before subscription writes; slow observe/post/all-subscription writes consume
that same total. Setup and independent five-second cleanup are separate.
Observe keeps its per-event idle timeout; unrelated complete events start the
next idle wait as before. Stream without --timeout remains unbounded and
cancellable. Stream total expiry now consistently ends successfully unless
cleanup fails; its old idle-read expiry could fail while between-frame expiry
succeeded. This timing/exit change is deliberate.

One pinned signal future is shared through setup/capture. Ctrl+C during
Stream/Observe ends normally subject to cleanup, while other commands return
typed cancellation. Ready control wins before another ready event is emitted.
Event filtering/order, JSON/human success lines and original count0 behavior
remain: Observe emits zero; Stream stops after its first event. Yielding occurs
at complete frame/output boundaries. A partial-frame stop drops an invalid
transport without unsafe Shutdown. Cleanup-primary retention remains as above.
Synchronous output/decode and running blocking workers remain cooperative;
there is no async Drop, ACK, rollback or safe-retry promise. Actual Linux SIGINT
at an owned synthetic discovery endpoint and injected writer errors are host
evidence; device stdout/signal/cleanup acceptance remains pending. See
[notification CLI control acceptance](validation/notification-cli-control-2026-10-04.md).

Keep canonical command/flag names, aliases, defaults, selection and confirmation
behavior unless an explicit migration says otherwise. Adding an opt-in command
or flag must not alter an existing invocation. Never add implicit pairing,
installation, retry of side effects or destructive operations as a compatibility
fallback. Default bindings, access grants and Host/Origin policies follow the
existing [management](management-access.md)/[HTTP](http-boundary.md) contracts.
The S02 GET management tombstones and their deadline are a documented migration,
not permission to restore GET mutation.

`ios list` defaults to one JSON array; entries have string `udid`, integer
`device_id`, string `connection_type` and integer `product_id`. No devices is
`[]`. `--no-json list` keeps its human rows and `No devices connected.` empty
message. The canonical usbmux `ProductID` now supplies the existing product_id
field; absent metadata retains zero. Older accepted `ProductId` is also read.
This is a decoding correction: the JSON field/type and command remain unchanged,
but consumers must not rely on previously erroneous zero for advertised values.
Host fake-usbmux observations do not certify a physical device.

For every changed command, record its root shape, field names/types, optional
fields/null/empty distinctions, integer width/precision, Data/Date/Uid policy,
stream framing, stdout/stderr, exit and partial-result behavior. R04 preserves
legacy command-specific array/hex encodings; A02 supplies new structured errors
without silently replacing existing command results. Consumers of an expressly
extensible JSON schema should ignore unknown fields; that rule cannot be
retroactively assumed for an older schema. Introduce an explicit output/schema
version or opt-in form when changing an incompatible shape. Human prose and
diagnostic correlation are not machine classifiers.

`--error-format=human|json` is an additive global option, defaulting to human.
The opt-in v1 final-failure stderr schema has stable category/phase/exit and
explicit unknown effects; success stdout and original exits remain unchanged.
Select the format before invalid arguments. Additional fields in this new
schema are extensible; other stderr progress/stream records remain separate
pending work. See the [error contract](error-contract.md) for exact fields and
parser recovery. This does not modify existing HTTP/C/Python error schemas.

Protected manager/CDP/WebDriver router errors additionally expose fixed
`X-Ios-Error-Schema-Version`, `X-Ios-Error-Class`, `X-Ios-Error-Phase` and
`X-Ios-Error-Effects` response headers. Existing statuses/JSON bodies and success
formats remain unchanged. Native typed failures and named admission outcomes
supply the category; unknown sources retain Unknown. Approved origins can read
these headers through explicit CORS exposure, with admission unchanged. The
[error contract](error-contract.md) defines version/codes, fixed phases and
unknown effects, plus router/upgrade/parser boundaries. CLI error-format does
not control this HTTP profile.

[R04 encoding/output migration](plist-json-contract.md) retains historical
Data/Date/Uid profiles. Failed result stdout now returns 1 instead of a possible
panic/101; failed help/version stdout returns 1 instead of silent success/0.
An already-unsuccessful command retains its status if stderr reporting also
fails. Output failure can follow completed side effects and does not imply
rollback or permission to retry.

## C ABI and ownership

Do not change a current symbol, calling convention, parameter type/order,
`size_t`/integer width, status numeric value, enum discriminant or `IosDevice`
field/layout. Adding fields to that array element changes indexing/layout and
requires a new versioned structure and functions. New interfaces should use
opaque handles or size/version-tagged new structures with explicit initialization
and output lengths. New mode inputs can use validated integer types; the existing
C enum accepts only its declared values, so arbitrary integers are not a safe
way to test or negotiate future modes.

Allocate/free across the matching library boundary. Device arrays use
`ios_free_devices` with their returned count; strings use `ios_free_string`;
device/tunnel handles use their matching close function. NULL teardown is
permitted as documented/tested. Closing a non-NULL handle frees it: the caller
must clear its pointer and must not close/use that dangling pointer again.
Existing APIs do not promise an idempotent repeated close on an already-freed C
pointer. Metadata/error output reset follows each header function; keep the
original nonzero status and release only successfully acquired resources.
Calls are synchronous and may use the shared runtime; no interruptible C call,
thread-safe handle mutation or orderly Apple teardown guarantee is added by this
policy. A03/L01 must explicitly design and verify those extended contracts.

The additive `const char *ios_last_error_class(void)` returns a borrowed static
category for the caller OS thread's last int-status operation, without changing
existing symbols or status values. Never free/modify that pointer. Success
clears to `none`; void initialization/cleanup, uint16_t ports and the getter do
not replace the observation. Typed native sources are classified before numeric
conversion; opaque/lost sources stay unknown. Read it on the same thread and
keep a fallback for future codes. The [error contract](error-contract.md) records
the precise lifetime, status mappings and partial/cleanup boundaries.

The additive `ios_list_devices_checked` uses the same output layout and free
function. It rejects an embedded NUL in any received UDID or connection type
with `IOS_ERR_CORE`/`protocol` and NULL/0 outputs, returning no partial array.
Valid and empty inputs succeed. The old listing symbol retains its historical
skip behavior; callers requiring detectable encoding failures migrate to the
checked symbol and inspect the returned status/class. This validates the
received list's C projection, without certifying upstream discovery completeness.

The additive `ios_tunnel_rsd_port_checked` and
`ios_tunnel_userspace_port_checked` return int status with a caller-owned
`uint16_t` output. NULL input gives `IOS_ERR_NULL`/`invalid_input`; a zero cached
port gives `IOS_ERR_STATE`/`precondition`; success writes a nonzero port and
clears the category. A non-NULL output is cleared to zero before validation.
The old uint16_t queries keep their zero fallback and do not update the
category. Consumers needing distinct NULL/missing/success outcomes can migrate
to the additive symbols with a library that provides them. These are cached
metadata queries, requiring no runtime/network work; neither success nor a
missing value establishes support, mode, current reachability or cleanup.
See the [error contract](error-contract.md) for pointer/thread and scope limits.

The additive `ios_runtime_init_checked` returns the cached shared initialization
status, classifying ordinary native IO as `IOS_ERR_RUNTIME` and caught unwind as
`IOS_ERR_PANIC`/`unknown`. It does not retry a cached failure. The legacy void
initializer now catches unwinds before the C boundary while preserving ordinary
initialization/category behavior. Five added status entries include this
initializer and checked device/tunnel/string/list releases. All old signatures,
layouts and status values remain available.

Checked releases take caller owner storage, clear accepted pointers before
release and report local status, including poisoned handle state or caught
unwind. NULL owned resources succeed; NULL required owner storage fails.
List release requires both outputs and leaves both unchanged if either is NULL.
After consumption, a failed status never authorizes retry through a saved alias.
Read/save the original operation's status/category before checked cleanup,
because even successful no-op cleanup resets the category. The existing void
cleanup APIs preserve it. Repeated release of the cleared owner is a safe no-op,
not repeated close of a dangling pointer. Local release is separate from task
join/Apple cleanup; L01/A03 retain those stronger lifecycle contracts. The
[error contract](error-contract.md) defines exact pointer/count/thread limits.

## Python API and ownership

Preserve module/distribution names (`ios_rs` / `rust-ios-device-tunnel`), factories,
parameter names/order/kinds/defaults, attribute types, exception types and
context behavior. Factory resource classes are not directly constructible.
New optional arguments should avoid shifting existing positional parameters;
use keyword-only options when suitable from their first introduction. Compatible
method additions need a name-collision and return-type review. Keep the abi3
Python 3.9 floor and separate host binding/link tests from installed wheel/sdist
checks on the minimum and a representative newer runtime.

`Tunnel.close()` requests closure and may retain the device while an installed
asyncio patch still owns it; metadata is a snapshot. A patch restores only its
own nested installation, with the last restore reinstating original
`asyncio.open_connection`. The current patch is process-global and should be
coordinated by the embedding application; it is not a concurrent per-task
network context or a generalized async SDK. Repeated Python close/restore and
patch ownership have host tests, not a promise of a timely Apple teardown ACK.
Native factory failures keep safe RuntimeError, invalid tunnel mode keeps
ValueError, and unavailable asyncio proxy keeps RuntimeError, including their
existing single-string args. Additive `ios_error_schema_version` (1),
`ios_error_class`, `ios_error_phase` and `ios_error_effects` attributes provide
typed classification and explicit effect uncertainty without changing public
signatures. Consumers allow future codes/attributes and retain a fallback for
exceptions outside this profile. The [error contract](error-contract.md) records
allocation/callback/runtime boundaries, safe causes and host-only acceptance.

Ordinary runtime-initialization IO failure now produces a safe RuntimeError with
typed metadata and `initialize runtime` phase, replacing the former expect/panic
path and avoiding lazy-lock poisoning on that ordinary error. Existing factory
signatures, success values, native operation errors and invalid-mode precedence
remain. Initialization/blocking work still release the GIL. Applications can
catch RuntimeError for this exceptional failure; remaining interpreter/panic
boundaries stay outside the profile. The release must account for this documented
exception-type behavior change under the version/migration rules below.

## Versions, migrations and checks

The workspace currently declares 0.1.13; that version has already been published.
Do not overwrite it. Choose an unused, coherent version across the four crates,
header/artifacts and Python package when D02 prepares a release. For this
project's pre-1.0 line, a reviewed patch should preserve maintained profiles;
source/ABI/output-breaking changes require a new minor line and explicit
migration. Before 1.0, the catalog and release notes continue to mark unsupported
or experimental workflows. Security corrections may require an intentional
behavior change; record its trigger, scope, safe replacement and deadline rather
than promising an unsafe compatibility bypass. Every exception requires an
explicit review and release note; general experimental status is not a blanket
license for silent breakage.

Each public change must state old/new usage or output, affected callers and
feature/platform scope, deprecation period/aliases or versioned replacement,
ownership/cancellation/error differences, exact tested commit and remaining
evidence. Preserve historical release notes. The [PR template](../.github/pull_request_template.md)
and [roadmap](engineering-roadmap-2026-09-30.md) carry these records.

`tests/public_contract.rs` is an external-crate consumer using root imports,
struct literals, exhaustive enum matching, service metadata, codec roundtrip,
unpolled async signatures and feature alias identity. It runs at default/full
features; targeted alias mode also runs explicitly. Existing compile_fail docs
retain private-module encapsulation. `validate.py compatibility` exercises the
actual CLI against bounded owned USB/Network/empty fake-usbmux responses in JSON
and human mode; only safe counters are printed. Linked C smoke checks the original
14 signatures plus the additive category getter, checked listing, two checked
port queries and five initialization/cleanup entries, statuses/layout/null/UTF-8
ownership failures and owned malformed/checked ListDevices replies with consuming
checked free. Cached-port, allocated string, no-device handle/poisoned state and
caller-thread transitions are exercised by Rust-owned fixtures; the consuming
helper also has counted destructor/unwind coverage. These are projection/local
ownership tests, not physical tunnel or Apple teardown acceptance.
Installed Python smoke checks factories/signatures and patch/resource exposure,
invalid mode with an owned endpoint trap and a real malformed owned ListDevices
reply. Its child/peer deadlines and bounded request permit no device-service
routing; this is host evidence, not an actual device workflow.
These are representative regression profiles, not an exhaustive semver checker
or ABI proof across all architectures. Platform release checks, A02/L01 and
representative device evidence remain required before 1.0 readiness.

For the experimental CDP page bridge, a timeout that invalidates the underlying
WebInspector client now remains the returned primary error. It previously
became a local unusable-connection Protocol error after another iteration.
Healthy empty idle, successful target messages, normal close, API/CLI grammar
and access policy stay. The private source/handler fixtures verify host behavior,
with no new public source selector or physical-device/browser acceptance.
This error-behavior correction needs pre-1.0 release review; setup/request totals,
cleanup results and joined server/browser ownership remain pending. See
[CDP page lifecycle](validation/cdp-page-idle-lifecycle-2026-10-05.md).

CDP discovery/attachment and WebDriver runtime setup now consume one configured
total, including lock/connection, startup, discovery and attachment/context
creation. The timeout previously restarted across those stages. Full lists
and automation discovery use a500ms quiet window inside the total; insufficient
totals can fail even when an individual stage would previously complete.
Exact CDP targets proceed when actually discovered, and absent targets wait
within that total. Successful owners clear the setup cap before existing active
operations; no stale setup allowance becomes a request limit. Incomplete browser
attachment now drops without guessing teardown. These timing/selection/failure
corrections need pre-1.0 release review. Public grammar/output/API/access policy
stay; private zero fixtures do not prove CLI listener admission. Server startup,
signal, publication, active request/output, primary cleanup and joined task
contracts remain pending. See
[WebInspector server setup](validation/webinspector-server-setup-2026-10-05.md).

The direct CDP page bridge now retains teardown errors after normal WebSocket
shutdown and after a primary failure. Cleanup has an independent 2s budget;
previously it used the active timeout and discarded the result. A successful
page close can therefore produce a cleanup failure in the existing diagnostic
path. Primary classification and native sources survive when both stages fail.
This error/timing correction needs pre-1.0 release review. Wire selectors, page
messages and public grammar stay. Constructor cleanup, outer lease cancellation,
server shutdown and joined browser/stored tasks remain pending; this is not an
async Drop guarantee. See
[CDP page cleanup migration](validation/cdp-page-cleanup-2026-10-05.md).

WebDriver runtime stop now closes its owned forwarding socket after the
Automation browsing context, borrowing the original session without cloning
command IDs. Both steps share the existing stored100ms allowance. Cleanup starts
once per runtime; success, failure or caller cancellation cannot authorize a
second attempt. The runtime preserves native and dual cleanup errors. HTTP
DELETE grammar, response shape and its existing result-discard policy stay;
routed failure handling remains pending. This forwarding/timing correction needs
pre-1.0 release review and device evidence. See
[WebDriver cleanup migration](validation/webdriver-cleanup-2026-10-05.md).

Direct CDP page policy expiry/reload now stops page work, returns typed
cancellation and waits for existing independent2s cleanup before release. The
connection can remain owned during that cleanup; no further business messages
are processed. Previously outer lease cancellation dropped the handler without
cleanup or diagnostics. Revocation now uses the existing safe diagnostic line,
retaining primary and cleanup failures without sending new WebSocket errors.
This timing/diagnostic correction needs pre-1.0 release review. Admission,
tickets, policy fields, successful messages and public grammar stay. The earlier
direct-page lease gap is covered; generic caller/server Drop, browser children,
stored-session joins and device evidence remain. See
[CDP page lease migration](validation/cdp-page-lease-2026-10-05.md).

CDP browser pages now use independent2s cleanup instead of the active timeout;
explicit removal joins the child and reports retained failures through existing
safe diagnostics. Browser loop exit stops remaining children together under
one2s total, then aborts and joins every unfinished handle. Successful detach
replies, events, routes, admission and public grammar stay. Private constructor
return ownership changes only inside the CLI; native and dual failures are not
exported as unsafe messages. These timing/diagnostic corrections need pre-1.0
release review and device evidence. Browser lease/server/caller Drop and stored
registry joins remain pending. See
[browser task migration](validation/cdp-browser-tasks-2026-10-05.md).

CDP browser policy expiry/reload now stops business work and waits for common2s
cleanup before release. A child removed during detach remains owned when lease
cancellation interrupts its cleanup; completed joins and known native errors
survive. Previously outer Principal.run dropped the parent and aborted these
children without awaited cleanup. Revocation now uses existing safe diagnostics
with typed cancellation, preserving cleanup failures privately. No successful
reply, event, route, policy/admission field or public grammar changes. This
timing/diagnostic correction needs pre-1.0 release review and device evidence.
Generic caller/server Drop and stored-session joins remain. See
[browser lease migration](validation/cdp-browser-lease-2026-10-05.md).

### WebDriver DELETE failure migration (2026-10-05)

Successful and repeated DELETE retain200 and `{"value":null}`. Cleanup failure
now uses the existing HTTP500/safe unknown-error body and schema1 machine
headers. Previously it was hidden behind successful deletion. The100ms total
starts at handler entry, covers admission removal, runtime mutex and original
Automation context/forwarding cleanup; it does not include preceding HTTP
authorization middleware or response delivery. Known business error remains
primary through cleanup expiry. Routes, parameters, successful payloads and
public Rust/C/Python surfaces stay. Review this error/timing change for the
pre-1.0 release; real-device evidence and in-flight request/registry ownership
remain pending. See [DELETE migration](validation/webdriver-delete-2026-10-05.md).

### Stored session watcher ownership (2026-10-05)

Registry removal/replacement now issues watcher abort before waiting for its
join. The retired owner remains in the store if that wait is cancelled;
subsequent registry calls reap finished actors. A revoked actor moves its own
handle to the store, then finishes cleanup without aborting/joining itself.
Original session middleware, private registry signatures, scopes, routes,
successful responses and public Rust/C/Python surfaces stay. WebDriver DELETE's
existing100ms total now also includes this map-removal join wait. This timing
correction needs pre-1.0 release review and device evidence. Generic Drop,
legacy callback errors, in-flight request Arcs and server shutdown remain
separate. See [watcher migration](validation/session-store-2026-10-05.md).

### WebDriver revocation cleanup timing/diagnostics (2026-10-05)

After retiring the entry, the watcher creates one checked100ms cleanup Budget
and passes it to the existing typed runtime helper. Runtime mutex and original
context/forwarding cleanup consume that deadline. The old unit log adapter and
competing outer timer are removed from this production path. Returned native
primary/secondary errors survive through the owned join/reap and existing safe
tracing. This timing/diagnostic correction needs pre-1.0 release review and
device evidence. Constructor, DELETE, routes, successful payloads, HTTP schema,
authorization and public Rust/C/Python surfaces stay. This total starts after
entry retirement; preceding map wait and server shutdown remain separate. See
[revocation migration](validation/webdriver-revocation-2026-10-05.md).

## L02 private CLI manager coordination

The [manager isolation profile](validation/tunnel-isolation-2026-10-06.md)
tests `e0f52671396e6408fc38921f78f73a256a45ce65`. Private device source,
per-device generations and four shared permits change CLI manager scheduling.
No Rust SDK/C/Python symbol, struct, enum, field, dependency or Apple protocol
change. Existing HTTP routes/payloads/auth stay; stopping an absent pending
start still returns404 while its request receives existing502/cancelled.
Manual start now shares15s after ticket acquisition across queue/permit/
connection/discovery/fallback/publication, and unmatched fallback retains native
IO. These observable timing/category changes require pre-1.0 release review.
Two protected HTTP fixtures and actual TunnelAgentState owned fake-device tests
are host evidence. Source/result hashes and all14 subprocess exits are audited;
parent9635 tool exit is unavailable after handle recovery. Automatic scans still
connect sequentially; physical/native joins/resource/L04 and all remaining
L01/L02/L03–L04/Q04/classes5–9 conditions stay.
