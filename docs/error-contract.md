# Error and machine-output contract

A02 is in progress. Additive Rust classification, opt-in final CLI machine
errors, Python factory/proxy metadata, caller-thread C categories and protected
router HTTP headers are implemented; the roadmap keeps remaining HTTP/Python/C
boundaries, stream semantics, probe observations
and remaining information-loss fixes pending. R04's accepted
[stdout/encoding migration](plist-json-contract.md) remains independent.

## Rust classification

`ios_core::{classify_error, ErrorClass}` and `CoreError::class()` inspect typed
errors and their original sources. Existing public error enum definitions and
fields remain unchanged, including exhaustive CoreError matches. The explicit
migrations below describe returned-variant, Display and source-chain changes
where adapters previously discarded typed failures. ErrorClass is a new
non-exhaustive enum, serializes to a snake_case
string, and exposes the same stable code through `as_str()`. Callers must keep
a fallback for future categories. Human diagnostic text is not a classifier.

| Stable code | Typed evidence |
| --- | --- |
| `timeout` | IO TimedOut, Tokio Elapsed, or an existing service Timeout variant |
| `cancelled` | Caller-observed OperationCancelled marker or a cancelled Tokio JoinError; neither acknowledges device cleanup |
| `disconnected` | Named IO connection-reset/aborted/not-connected/broken-pipe categories or explicit closed/dead-connection variants |
| `service_missing` | XPC ServiceNotFound or the RemotePairing ServiceNotFound variant |
| `permission_denied` | IO PermissionDenied, AFC PermDenied, or typed pairing rejection |
| `protocol` | Existing Protocol/typed codec shape failures or invalid data; a codec's underlying IO failure takes precedence over its generic fallback |
| `invalid_input` | IO InvalidInput, credential parse, GPX/ZIP input failure, or typed test-plan input validation |
| `unsupported` | Existing Unsupported/LegacyUnsupported/FeatureMissing/deprecated-version/backend variants |
| `device_not_found` | Existing Core/usbmux DeviceNotFound |
| `not_found` | IO NotFound, missing pair record or AFC ObjectNotFound; it is not inferred to mean a missing service |
| `resource_limit` | Explicit codec nesting/resource-limit, clipboard Limit or named AFC resource statuses |
| `conflict` | Typed media-in-use, DDI conflict or AFC existing/busy object |
| `precondition` | Typed passcode-set or legacy WebInspector Disabled; Disabled does not prove a permission rejection |
| `service_failure` | An explicit service/device/status failure without a more specific known category; unknown wire status codes stay here |
| `io` | Other IO kinds, including EINTR and UnexpectedEof where the underlying object could be a host file |
| `unknown` | Unrecognized typed errors, opaque legacy failures or lost source information |

Classification reads at most 32 sources and never formats or matches an error
message. The first precise typed category wins. Generic codec/IO fallback lets
an underlying writer/reader failure retain its category; custom IO payloads are
visited directly. Cycles and deeper chains stop at the same bound. A category
alone never implies retry safety, rollback, absence of side effects or successful
protocol cleanup. Keep the original error available when caller-authorized
details are needed; automatic diagnostics still use the redaction boundary.

## Known information-loss boundaries

Existing Protocol strings cannot be reclassified by words such as "timeout"
or "permission denied". Existing Other strings remain Unknown. Several older
callers flatten error types into strings, and WebInspector startup maps some
timeout/closed-stream failures to Disabled. Those sources require an additive
API or an explicit migration; this classifier does not restore information
that was already discarded. IO UnexpectedEof and EINTR alone do not prove a
device disconnect or a cancelled operation.

`OperationCancelled::new()` is an additive error marker with private fields,
also available through Default. A caller uses it only after observing local
cancellation; it is not inferred from an Interrupted IO kind or prose. Existing
CoreError variants stay unchanged. WebInspector launch records this marker under
its existing cancellation context when Ctrl+C completes successfully; failed
signal registration retains its original IO error, and the outer timeout keeps
Elapsed. The opt-in final CLI class consequently becomes `cancelled` instead
of the former string error's `unknown`. Exit 1, safe human text, success output,
unbiased selection, timeout and caller-owned future scope remain unchanged.
Host ready-signal/owned-pending tests prove these source and drop boundaries;
they do not validate actual OS Ctrl+C against a device or orderly Apple cleanup.
An additional actual CLI fixture sends SIGINT to its owned process while its
sole owned usbmux peer withholds a ListDevices reply. It observes exit 1,
`cancelled`/unknown effects and peer EOF, before any pairing/device service.
This validates the host signal/error path and local socket release only.

The CLI connection, application-list, launch and Safari-launch deadline wrappers
retain Tokio Elapsed through anyhow context. An expired shared operation budget
retains a typed WebInspector Timeout under its existing human context. These
changes preserve timeout values and exits. They do not establish
cancellation/cleanup acknowledgement.

`WebInspectorClient::start_preserving_errors(timeout)` shares the existing
identifier-report/event loop and one deadline with `start(timeout)`, but returns
original IO errors and the existing typed Timeout instead of remapping them to
Disabled. The old `start` and WebInspectorError variants remain available with
their old behavior. Elapsed timers inside Core still become the existing
Timeout(Duration), rather than carrying a Tokio Elapsed source; response timeout
duration is the remaining phase budget, while a blocked report write uses the
original startup budget. This API restores the startup IO source, without
inventing a device setting or unsupported reply from a timeout or EOF.

CLI tasks, CDP discovery/page/browser startup and WebDriver runtime construction
now use the preserving method. A closed stream can consequently yield an IO
category in existing safe diagnostics instead of opaque Disabled correlation.
Existing failure exits/HTTP statuses/error body shapes, successful output and
wire frames stay unchanged. Migrate Rust callers by selecting the new method
and inspecting WebInspectorError or classify_error; callers needing the legacy
mapping can keep start. Neither method promises retry safety after a partial
write, a cleanup acknowledgement or that AutomationAvailability::NotAvailable
means Web Inspector itself is disabled.

`RsdFeatureEvidence` and the additive `feature_evidence` methods on
ServiceDescriptor/RsdHandshake describe a successful directory snapshot:
Advertised, NotAdvertised, MetadataUnavailable or ServiceMissing. Exact and
canonical-then-shim queries preserve the existing resolution order. An empty
legacy feature vector gives MetadataUnavailable; it cannot distinguish an
omitted list from a deliberately empty list. The old `supports_feature` keeps
its permissive behavior. NotAdvertised is not an explicit device-side
unsupported response; Unsupported errors remain a separate typed outcome.

`ConnectedDevice::initial_rsd_probe()` now borrows the connection owner's initial
observation without IO or retry. Its extensible state is `not_attempted`,
`available` or `failed`. Unattempted probes distinguish `tunnel_skipped` from
`direct_probe_feature_unavailable`; failures retain the CoreError reaching the
owner and offer an optional typed error class. Debug prints only fixed state,
skip reason and class, while explicit Rust callers can inspect the original
error/source. Diagnostic reporting must still follow the redaction policy.

`rsd()`/`into_rsd()` retain their legacy optional initial directory. A successful
directory can independently report ServiceMissing or MetadataUnavailable;
neither a skipped probe nor a failed request is an explicit device-side
unsupported response. Empty feature lists retain their permissive legacy
behavior. The added observation is historical: later on-demand retries do not
update it or the old cache, and it cannot certify current reachability or device
cleanup. Later service calls now retain their retry failure independently, as
described below; this getter does not become a mutable latest-probe report.

The private userspace-proxy dial adapter now retains native IO instead of
flattening it to XpcError::Tls text. RSD/H2 bootstrap and read/write adapters
also retain their typed sources as described below; the observation cannot
recover information already discarded by other adapters.
Existing public error enum definitions, retry counts, deadlines, bootstrap
frames and permissive feature rules remain unchanged. Read the initial state before interpreting a missing directory;
`rsd() == None` alone continues to conflate skipped and failed initial discovery.

On-demand RSD service calls keep the existing five attempts with 500 ms between
attempts, inner bootstrap budgets and fresh transports. When all completed
probes fail, the owner returns the final original CoreError/source instead of
the former `Unsupported("RSD handshake failed after retries")`. Earlier failures
are released; this is not an aggregate history. Owner preflight failures still
return immediately, and success stops retries. A skipped direct probe without
the local mdns feature returns Unsupported with that local prerequisite; it
does not prove a device-side rejection. No-tunnel behavior remains unchanged.

A missing canonical and shim service in a successfully obtained directory now
returns `CoreError::Xpc(XpcError::ServiceNotFound(requested_name))`, with class
`service_missing`, instead of Unsupported. Missing feature metadata and
unadvertised features keep their separate evidence semantics. The class concerns
that directory snapshot, without proving the service is absent for all routes
or currently unreachable. Both cached and newly discovered directories follow
the same rule; no service socket is opened for a lookup miss.

Migrate exhaustive callers that treated every former Unsupported as a device
capability decision: inspect the native class/source, keep a future fallback,
and do not retry side effects merely because discovery failed. Enum definitions,
signatures, successful frames/output, initial observations and cache behavior
are unchanged; returned failure variants and safe diagnostic correlation can
change. CLI screenshot fallback accepts the same specific missing service names
through the typed variant, retaining its legacy compatibility cases. XPC service
reconnector dials also retain native IO instead of a TLS string. Stronger
lifecycle/reconnection guarantees remain work.

RSD and RemoteXPC H2 setup, bootstrap, header/body reads, message writes and
cryptex transfer writes now wrap the original H2Error in the existing
`XpcError::Io` variant instead of formatting it into `Tls(String)`. A private
error payload preserves the fixed operation context and original source chain.
Native IO kinds remain intact; H2 protocol failures use InvalidData and retain
their typed protocol source. The wrapper's Debug exposes only fixed context;
explicit Display/source access still requires the diagnostic redaction policy.

Migrate callers matching every H2 failure as Tls or parsing Display: inspect
class/source and retain a fallback. Native timeout, permission and disconnect
failures can now report their actual category instead of protocol; Display gains
the existing IO variant's prefix, and safe diagnostic correlation can change.
Protocol/input strings that have no original source remain their existing Tls
variant. Public enums/signatures, wire frames, retry counts/budgets and pending
memory limits are unchanged. Host faults cover setup, bootstrap/write, header
and partial-body reads through all three connection receive methods, plus a
public XpcClient consumer. They establish local transport drop and retained
sources, without certifying safe reuse/retry after a partial operation or device
cleanup. Other lossy adapters and stream/binding boundaries remain pending.

## Machine output and partial results

Current CLI success formats and failure exits remain as recorded by R04.
`ios --error-format=json <command>` opts into one compact, newline-terminated
final failure object on stderr. `human` is the default and retains existing
safe diagnostic text. The option is global; put it before invalid arguments
so Clap can recognize the selection before its error. Unrecognized/invalid
format selection safely retains human errors. Recovery uses the same parser
without help/version early display; it never scans positional values or error
text to choose a format. Successful commands keep their existing stdout and
produce no final failure object. Help/version use normal Clap success output.

| JSON v1 field | Meaning |
| --- | --- |
| `schema_version` | Integer `1`; consumers should retain a version/fallback branch and ignore additional fields in this new extensible schema |
| `error.class` | Stable ErrorClass code from typed sources; a typed CLI confirmation guard maps to `precondition`, opaque strings remain `unknown` |
| `error.phase` | Parser's canonical root command or fixed `argument_parsing` / `help/version output`; no user command value is copied here |
| `error.diagnostic` | Opaque safe diagnostic/correlation; do not parse it for a category or assume correlation persists across processes |
| `error.exit_code` | Integer `1` for operation/help-output failure or `2` for argument failure; matches the original process status |
| `error.confirmation_required` | Boolean from a typed confirmation guard observed within 32 sources; false means no such guard was observed in that bound, and grants no confirmation or retry |
| `error.effects` | `not_started` for argument parsing before dispatch, `unknown` for operation/output failures; failure alone does not establish rollback, retry safety or completed cleanup |

Argument failures carry `invalid_input` (or an underlying IO class for a typed
Clap IO failure). IO on a closed result/help pipe may classify as `disconnected`;
that code alone does not identify a device as the disconnected object. Stderr
reporting remains best effort after failure: a closed stderr retains the original
nonzero status and may prevent delivery of the envelope. Ordinary progress,
tracing and stream callbacks may still emit other stderr records; this option
does not yet make the entire stderr stream JSON. Partial stdout may already
exist. Existing HTTP and C statuses retain their shapes. Additive HTTP headers
and C/Python category queries are described below; post-upgrade frames and
remaining adapters still need their own acceptance.

Both the human confirmation hint and machine field now inspect at most 32
sources, matching the classifier's bounded interpretation. Normal guard errors
and 24-layer contexts retain the hint. A guard hidden beyond the bound yields
no hint and Unknown classification; callers must not infer permission from its
absence. Previously the hint scan could continue indefinitely through a cyclic
source. This bounded diagnostic change does not alter the operation's actual
confirmation check or authorize any device action.
Existing safe diagnostic messages remain useful for correlation but must not
be parsed as stable natural-language categories.

Stream framing, ordinary stderr progress/callback failure, partial-operation
status, and JSON commands that currently emit human status text remain under
review. A partial stream or an output error can follow completed operations;
no rollback, automatic retry or device cleanup acknowledgement is promised.
L01 retains the stronger lifecycle/deadline acceptance.

`ios syslog` success stdout is newline-delimited JSON by default: one compact
object per matching relay message, retaining the existing parsed/raw fields.
Embedded newlines and quotes are escaped inside that object. `--no-json` retains
raw bytes; `--parse` retains its formatted line and added newline, including
existing extra newlines when the raw message already ends with one. Each emitted
record now checks both write and flush. An output failure can leave a partial
final record, so consumers must validate framing and inspect the process status.

Relay read errors, including EOF before a chosen stopping condition, now return
through the normal final CLI failure path with exit 1, rather than printing a
stream diagnostic and returning exit 0. Native IO/source/class are preserved;
UnexpectedEof remains `io` under the classifier's existing rule. Unterminated
relay messages are not emitted, and previously emitted records remain stdout.
The final failure uses safe human text or the opt-in JSON stderr envelope with
phase `syslog` and effects `unknown`. The existing streaming progress line stays
on stderr; its write/flush now returns IO failure instead of panicking.

The existing `--count` matching-record cap and user-selected `--timeout` capture
window still end successfully. That window starts after service setup; its
expiry is a chosen collection stop, distinct from a native reader TimedOut.
An unbounded listener gains no new idle timeout. Controlled host readers/outputs
exercise partial JSON, EOF, native failures, count/filter stops and output/flush
failure; an owned idle duplex observes capture expiry and peer EOF. This proves
local stream drop, without certifying physical relay/signal behavior, connection
reuse, rollback or device cleanup. Migrate scripts that relied on relay failure
as exit 0 to preserve valid prior records and handle the nonzero result. Other
streams and binding/lifecycle boundaries remain pending.

The relay's 1 MiB limit applies to message bytes before the NUL delimiter and
before vis/UTF-8 decoding. The cumulative check now also covers the final chunk
containing NUL, before copying it into the message buffer. An exact-limit record
is accepted; one extra byte is InvalidData/`protocol`, whether or not the final
chunk contains its terminator. Previously a terminated final chunk could bypass
the check. CLI syslog returns this error with prior NDJSON retained and no
oversized record emitted. Discard the reader after a size error; this fix does
not establish error/cancellation-safe reuse or a limit on the serialized JSON's
size after decoding/escaping. L01 retains connection invalidation/reuse work.

`apps install`, `upgrade`, `archive`, `restore`, `kill`, `signal`, `launch` and
`uninstall` now honor global JSON mode instead of printing human status lines.
Each emits one compact newline-terminated result after its existing service and,
where present, staging-cleanup checks. New records contain integer
`schema_version: 1`, an `operation` tag and the fields below. Preserve a version
and unknown-operation fallback, and ignore future extra fields. `--no-json`
retains the prior human bytes. Existing list/show/install-record/processes/roots/
spawn/icons/monitor JSON shapes stay unchanged.

| Operation | New result fields |
| --- | --- |
| `install` | `path` and boolean `streaming` |
| `upgrade` | `path` |
| `archive`, `restore`, `uninstall` | `bundle_id` |
| `kill` | unsigned `pid` |
| `signal` | unsigned `pid`, signed `signal` |
| `launch` | `bundle_id`, `pid` (null if the existing service result supplies no PID) |
| `pkill` | `pattern`, `signal`, `matched_count`, `completed_count`, `complete`; per-process records additionally contain `process: {pid, name}` |

`apps pkill` JSON is NDJSON. After each successful signal call it emits a record
with `complete: false`, that process and the number of calls completed so far.
After all matching calls succeed it emits one summary with `complete: true` and
no process field. No matches yields only the zero-count complete summary and no
signal calls. Counts describe successful signal requests, not observed process
termination. ASCII case-insensitive matching, process order and per-process
connections are unchanged. The human summary keeps its old wording, including
"Killed" for a selected signal other than SIGKILL.

A request failure preserves prior records and emits no complete summary. An
output failure also stops before the next process and may leave a partial final
record. Final errors retain their native source, exit 1 and unknown effects;
there is no rollback or automatic retry guarantee. Even a valid result can be
followed by a flush error. Result data intentionally includes requested paths,
bundle identifiers and process names; it is distinct from safe diagnostics.
Install/upgrade can succeed on the device before later cleanup/output failure,
so absence of a success record does not establish absence of device effects.
Streaming-install stderr callbacks remain a separate pending IO boundary.

Application fallback accepts the existing typed ServiceNotFound only for the
canonical appservice and its shim, retaining old Unsupported compatibility
cases. Native timeout/permission/refusal/broken-pipe or another missing service
does not select fallback. Ten DTX adapters now retain sources through fixed
anyhow contexts; install-record/monitor wrappers retain Tokio Elapsed, and
signal prerequisites keep the original routing error under context. Error enum
definitions, deadlines, successful wire calls and SIGKILL-only Instruments
fallback are unchanged. Returned error source/class, Display context and safe
correlation can change. The output/ordered-call fixtures use owned callbacks;
they do not certify physical mutation, DTX fallback or complete CLI service
startup workflows. Other JSON commands, callbacks and binding/lifecycle boundaries
remain pending.

## Python exceptions

The existing RuntimeError from `ios_rs.list_devices()` and `start_tunnel()`
retains its safe single-string `args`, with category selected from the native
typed error before converting to Python. Invalid tunnel mode keeps ValueError
and its exact message; `Tunnel.asyncio_proxy()` without a userspace proxy keeps
its existing RuntimeError. These errors gain four additive attributes:

| Attribute | Meaning |
| --- | --- |
| `ios_error_schema_version` | Integer `1`; allow future versions and additional attributes |
| `ios_error_class` | Stable ErrorClass code; consumers need a fallback for unknown/future codes |
| `ios_error_phase` | Fixed `list devices`, `start tunnel`, `initialize runtime`, `tunnel mode` or `asyncio proxy`, without user values |
| `ios_error_effects` | `unknown` for native operations; `not_started` for rejected mode or unavailable proxy before that operation |

No public factory/method parameter is added. Existing `except RuntimeError` and
`except ValueError` callers keep working; callers can select a category by
attribute instead of parsing diagnostic text. Invalid mode fails before runtime
initialization or device discovery. A proxy precondition failure does not retain
an asyncio patch. Unknown effects never grant retry, rollback or cleanup safety.
Original Rust sources remain available to native callers before this conversion;
they are not exported as unsafe Python causes or diagnostic payloads.

Ordinary Tokio runtime-initialization IO errors now return this safe RuntimeError
profile with phase `initialize runtime` and unknown effects, before device work.
The original IO error is cached instead of triggering the former expect panic
and poisoning later initialization attempts. Initialization and device blocking
work still execute inside allow_threads; invalid mode still fails first. This
changes the exceptional initialization path from a panic boundary to the mapped
built-in exception. Success and existing operation-error args/signatures remain.
Controlled host tests inject a typed failed initialization result; they do not
induce real thread/memory exhaustion or certify runtime cleanup under that failure.

If metadata allocation/assignment fails, that Python exception is returned with
the original safe Python exception as its cause. The operation remains failed;
complete metadata delivery is not promised under allocation failure. Python
argument extraction errors, interpreter/allocation errors, callback exceptions
and other runtime-internal panics are outside this mapped
profile and still need their own review. Absence of attributes does not mean
success or an unknown side effect was rolled back. The host tests and installed
owned ListDevices peer certify this boundary, not physical tunnel startup or
Apple teardown.

## C status categories

`ios_last_error_class(void)` is an additive symbol returning the stable ErrorClass
code of the calling OS thread's last function returning an int status. The
borrowed NUL-terminated string is static while the library remains loaded;
never modify it or pass it to `ios_free_string`. Read it on the same OS thread
as the operation, before another int-status call replaces the observation.
Each such call resets its prior category before work, records the native typed
source on failure, and clears it to `none` on success. `none` also describes a
thread before its first int-status call. Runtime initialization, void cleanup,
uint16_t port queries and the getter itself do not replace it. The getter does
not initialize or require the Tokio runtime.

The existing fourteen symbols/signatures, statuses 0–8, mode discriminants and
IosDevice layout remain unchanged. Core/serialization errors are classified
before status conversion. Runtime initialization retains its original IO error
instead of a String and still returns status 6 on failure. NULL/UTF-8 rejection
gives `invalid_input`, unavailable/poisoned handle state gives `precondition`,
and a too-small caller buffer gives `resource_limit`. The existing ALLOC status
also represents an embedded NUL, so it does not prove exhausted memory and
falls back to `unknown`. Captured int-call panic status 7 replaces any earlier
category with `unknown`; no panic message is returned by this getter. Remaining
opaque sources stay unknown. Consumers must retain a future-code fallback.

This query adds no interruptible C call, concurrent handle-mutation guarantee,
automatic retry or Apple cleanup acknowledgement. A `disconnected` IO class
does not identify which object disconnected. `none` describes a successful
numeric status, not a certificate of device cleanup or absence of partial
effects. Legacy void calls still report no outcome; additive checked entries
are described below. The owned linked-C malformed ListDevices fixture validates a real
library call, not device connection/query/tunnel acceptance. Native source
details are not exported as an unsafe C string.

`ios_list_devices_checked` is an additive int-status entry using the existing
IosDevice array layout and `ios_free_devices` ownership. If any received UDID
or connection type contains an embedded NUL, it returns `IOS_ERR_CORE` with
class `protocol`, clears outputs to NULL/0, and exposes no partial array. Native
IO/discovery errors retain their existing status/class mapping. Valid and empty
lists succeed and clear the caller's category to `none`.

The original `ios_list_devices` retains its historical behavior of skipping
unencodable records and returning the remaining rows, potentially an empty
success. Migrate to the checked symbol to detect these projection failures;
existing callers need no signature/layout change. Checking C encoding does not
prove completeness of the upstream discovery result or current reachability.
Controlled binary-plist peers exercise both real C entries and free successful
arrays. No device service, pairing, or Apple cleanup is certified by this check.

`ios_tunnel_rsd_port_checked` and `ios_tunnel_userspace_port_checked` add
int-status queries over the handle's cached port fields. Each non-null output
is reset to zero before the handle check. NULL input returns `IOS_ERR_NULL` with
`invalid_input`; a zero cached value returns `IOS_ERR_STATE` with `precondition`;
success writes a nonzero unsigned 16-bit port and resets the category to `none`.
They do not require a runtime, allocate a resource, send a request or update a
probe observation. A zero cache entry alone cannot distinguish kernel mode,
missing metadata, skipped/failed discovery or explicit device unsupported.
Neither zero nor nonzero establishes current disconnect or reachability.

The existing uint16_t queries preserve their signatures, zero fallback and
non-updating category behavior. Do not classify a legacy zero from the previous
int call's category. Migrate consumers requiring NULL/missing/success status to
the checked symbols and read the category on the same OS thread. Their library
must provide these additive symbols; old binaries retain the legacy API.
Outputs must not overlap the live handle, and concurrent close/use or freed
handle reuse remain unsupported. Rust-owned metadata tests exercise both actual
exported functions for missing/nonzero values, NULL output and category/thread
transitions; strict linked C verifies signatures and NULL clearing. This proves
the projection boundary, not a physical tunnel/proxy/RSD probe or Apple cleanup.
Other binding/source/lifecycle review remains pending.

`ios_runtime_init_checked` returns an int status for shared runtime initialization:
success clears the category, a cached ordinary native IO error returns
`IOS_ERR_RUNTIME` with its typed category, and a caught unwind returns
`IOS_ERR_PANIC`/`unknown`. The original IO result remains cached and is classified
without exporting its private Display text. Repeating initialization does not
retry a failed result. No device is opened. The legacy void initializer retains
its ordinary result/category behavior and now catches unwinds before the C
boundary. Catching an unwind does not suppress a process panic hook or recover
from abort/invalid memory. The global initializer panic path is source-reviewed,
not established by an actual process-level poisoned runtime fixture.

`ios_device_close_checked`, `ios_tunnel_close_checked` and
`ios_free_string_checked` take writable pointer-to-pointer owner storage.
A NULL owner pointer returns `IOS_ERR_NULL`/`invalid_input`; a valid owner whose
value is NULL succeeds as a no-op. A non-null owned pointer is cleared **before**
release. Handle state poisoning returns `IOS_ERR_STATE`/`precondition` after
consumption and local drop; a caught unwind returns `IOS_ERR_PANIC`/`unknown`.
The owner stays cleared on these failures. A consumed pointer or retained alias
must never be retried, used or freed. No API validates arbitrary/dangling pointers.

`ios_free_devices_checked` requires both pointer and count owner storage.
If either is NULL, neither is modified and the result is `IOS_ERR_NULL`/
`invalid_input`. Once both are accepted, the list pointer and count are cleared
to NULL/0 before release. A NULL list succeeds as a no-op and clears the count;
a non-null list requires its exact returned count, including a non-null allocated
empty list. The standard allocator/string ownership rules remain unchanged.
Owner storage must be writable, distinct where there are two outputs, outside
the allocation being released, and unavailable to concurrent use/close.

Every checked call uses the same caller-thread category replacement rule.
Read/save an operation's original status/category **before** checked cleanup;
even a successful no-op cleanup resets the observation to `none`. The borrowed
static code pointer remains valid after later int calls while the library is
loaded. Inspect release failures separately. Original void cleanup preserves
the category and signatures; its shared release routines retain prior behavior.
The checked symbols require a library providing these additive entries.

Success reports local release/no-op status. It does not acknowledge Apple
teardown, task joins, cancellation completion or absence of partial effects;
other owners can retain a ConnectedDevice. Failure after consumption does not
promise all resources were released or supply rollback/retry authority. Repeated
release is safe only through the now-NULL owner storage, never through a saved
non-null alias. Owned tests cover native initialization projection, actual
runtime success, allocated lists/strings, no-device handles and poisoned state;
the shared consuming helper's counted destructor fixture verifies clearing,
single drop and caught unwind. Strict linked C exercises actual ListDevices
allocations and checked freeing. These do not certify physical service/tunnel
teardown or actual global runtime resource exhaustion. Stronger L01/A03 contracts,
other sources/bindings and stream/callback boundaries remain pending.

## Protected HTTP responses

Manager, CDP and WebDriver router errors gain four fixed response headers,
without changing their existing HTTP statuses, JSON body shapes or safe
messages. Successful/informational responses do not gain these fields.

| Header | Meaning |
| --- | --- |
| `X-Ios-Error-Schema-Version` | String `1`; retain a future-version/fallback branch |
| `X-Ios-Error-Class` | Stable ErrorClass code from a native typed source or an explicit admission/issuer outcome; opaque/unobserved sources stay `unknown` |
| `X-Ios-Error-Phase` | Enum-selected `http_request`, `http_access`, `browser_ticket`, `webinspector`, `tunnel_manager` or `policy_reload`; never a submitted URL, selector or header |
| `X-Ios-Error-Effects` | String `unknown`; failure alone does not certify rollback, safe retry or completed cleanup |

WebInspector conversions and manager operation failures inspect the original
typed error before constructing the existing safe body. Both manager dial
timeout wrappers use the actual Elapsed class while preserving their original
message-only response. Access/Host/Origin refusal is PermissionDenied; ticket
capacity is ResourceLimit, issuer input rejection InvalidInput and unavailable
entropy Unknown. These are local named outcomes, not a classifier of arbitrary
HTTP status numbers. Named manager hop/upstream limits are ResourceLimit,
zero/missing connect input is InvalidInput, registry/tunnel lookup misses are
NotFound, and unsupported local transport/legacy GET routes are Unsupported.
Those local outcomes do not prove a missing daemon service or an explicit
device-side capability rejection. The transport guard is currently dormant:
every constructible StartTunnelConnectionType returns no unsupported message;
its header mapping is source-reviewed, with no fabricated runtime/device proof.
Other opaque failures and framework rejections
without a retained native source stay Unknown even when their prose/status
mentions timeout or a missing target.

The reviewed-response marker stays in server-only extensions. An incoming or
unreviewed outgoing header cannot bypass body sanitization or select a class.
Framework error bodies/headers are discarded as before, with status/Allow,
no-store and empty HEAD bodies retained. HEAD can still deliver machine headers.
Approved Origin responses expose exactly these four error headers to browser
fetch. Existing Host/Origin/credential and preflight admission remain unchanged;
unapproved origins gain no CORS access. Existing JSON/body consumers require no
migration; new callers read headers and allow future codes/versions.

This profile covers responses reaching the protected routers. TLS/HTTP parser
failures before routing, WebSocket frames after upgrade, remote client error
sources, remaining opaque decisions and stream/partial semantics remain
separate work. Missing headers are not proof of success. Native failures are
classified before boundary conversion; private original Rust chains are not
exported as unsafe HTTP data. Host/Chromium evidence does not certify a device
workflow or Apple teardown.
