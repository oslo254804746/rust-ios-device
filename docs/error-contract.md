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
| `invalid_input` | IO InvalidInput, credential/GPX parse, known non-IO ZIP package failure, or typed test-plan input validation |
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
Streaming-install stderr uses the checked observer contract below.

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

`apps::install_ipa_with_checked_progress` adds an optional
`CheckedProgressCallback` returning `io::Result<()>`. A callback error immediately
returns the existing `ZipConduitError::Io` with the original IO source and native
class, without reading another reply or reporting completion. The old
`install_ipa` and void `zipconduit::ProgressCallback` keep their signatures and
callback/completion behavior through the same transfer implementation. Error
enums, requests, progress fields, remote-error precedence and successful
transfer bytes are unchanged. Completion replies do not invoke either observer.
This API accepts returned observer errors; it does not catch callback panics.

CLI streaming install now checks the existing startup stderr line and each
progress write/flush. Progress stays stderr text with the same diagnostic ID
projection; global JSON success output keeps its versioned install result on
stdout. Startup output failure returns before payload transfer but after service
connection. Progress observer failure occurs after payload transfer and can
leave earlier lines or a partial last line; even a full line can precede a flush
error. No success stdout result is emitted on these failures. Final errors follow
the safe CLI exit-1 path, phase apps, with the native class and unknown effects.
Final stderr reporting remains best effort if stderr itself is unavailable.
A BrokenPipe/disconnected category for output does not identify a disconnected
device and supplies no retry/rollback authority.

After observer failure, discard the service connection. The core API borrows
its stream and does not close/join it; CLI dispatch returns through its existing
owner-drop path. Neither path certifies cancellation completion or Apple cleanup.
Owned tiny-IPAs/services drive the entire public install function, showing no
later completion read after callback failure, unchanged legacy/checked/no-observer
transfer bytes and completion behavior, and original remote/plist/IO failures
after progress. CLI writer fixtures exercise the shared checked writer and actual
safe error renderer; an external consumer keeps exhaustive error matching and
old/new callback signatures. These are host source/output/transfer fixtures,
not a paired-TLS CLI process or physical installation/teardown. Other callbacks,
JSON commands, bindings, other source adapters and L01/L03 remain pending.

Streaming install's local archive-open, entry-open and entry-read adapters now
retain the original `zip::result::ZipError` or `io::Error` under the existing
`ZipConduitError::Io`. Previously they flattened every failure into `Zip(String)`.
No enum variant or function signature changes; manually constructed legacy
`Zip(String)` still classifies as invalid input. Consumers matching `Zip` for
decoder failures must migrate to `Io` and typed cause inspection, or use
`classify_error`. Returned Display prefixes and safe diagnostic correlations
can change. Class selection uses typed variants/kinds, never error words.

| Original decoder cause | Returned kind / stable class |
| --- | --- |
| InvalidArchive, UnsupportedArchive, FileNotFound, InvalidPassword | InvalidInput / invalid_input; these describe the local package, without device support or service-missing inference |
| ZipError::Io or entry-read IO | Original native kind/class, including Timeout, PermissionDenied and disconnected; InvalidData becomes protocol, UnexpectedEof/Interrupted/Other remain io |
| Future non-exhaustive ZIP reason | Other / generic io fallback, with the original typed cause retained |

Fixed owner context is `open IPA ZIP archive`, `open IPA ZIP entry` or
`read IPA ZIP entry`. Owner/new wrapper Debug and CLI machine diagnostics keep
private data hidden; native Rust cause access intentionally retains original
payloads. Entry names are no longer inserted into read-failure context. A real
stored-entry checksum mismatch emits native InvalidData and now classifies as
protocol instead of the former flattened invalid_input. Locked zip 2.4.2 checks
local headers during archive construction, so a damaged header is an archive
phase error; a password-required metadata entry reaches the entry phase.

Both public install entries finish local extraction before writing InitTransfer
or payload, flushing, reading replies or invoking observers. Owned fixtures
verify these boundaries on malformed archive/header, password-required metadata
and actual CRC failures. CLI rendering of a real local decode failure keeps
phase apps, exit field 1 and unknown effects; CLI service setup happens before
local decoding, so this envelope does not claim the whole command never started.
Valid/empty archive entries retain names, order, contents and directory flags;
successful transfer bytes/keys and whole-file/in-memory extraction are unchanged.
These host faults do not prove physical installation, encrypted-payload support,
cleanup, stream reuse or new decompression/resource budgets. Other A02 source,
JSON/callback/binding work and L01/L03/L04/Q04 conditions remain pending.

## Connection and pairing native sources

Twenty connection/pairing adapters now retain original native objects instead
of flattened Other or Protocol strings, under existing CoreError::Io and a
private fixed-context payload. Four task-join paths retain JoinError, including
observed cancellation or blocking-worker panic. Two verification and two PSK
paths retain the original PairingError/PskTlsError, native IO kinds and precise
source categories. A PSK backend-unsupported error also retains its OpenSSL
stack as a standard source. Three local credential adapters retain typed host
identity/hex failures with InvalidInput. Two remote Base64 decoders, two received
public-key slice conversions and tunnel IPv6 parsing retain their native objects
with InvalidData/protocol. Four listener AEAD adapters retain the original AEAD
error: encryption preserves its unknown category and response authentication
failure is explicitly protocol, without guessing a permission rejection.

Private context construction takes a bounded category snapshot from the native
source. Classification uses that snapshot after any precise outer IO kind,
including Unknown; adding an IO container alone does not invent IO evidence
for an opaque crypto/backend/panic cause. Standard source inspection retains
the original object, even when that object's own older fields are already
strings. Construction reads at most the existing 32-source bound and does not
format text; subsequent classification of the context reads the cached value.
Wrapper Debug exposes fixed phase/category only, while original Rust Display
and source access can reveal private material. The native AEAD source requires
the existing dependency's std feature, now enabled with unchanged version and
crypto parameters; this workspace already requires std/Tokio.

Three other adapters retain original Tokio Elapsed under TimedOut: PSK dial,
RemotePairing control-channel dial and tunnel endpoint dial budgets. Values,
clock semantics, success and failure schedule remain. Fixed contexts replace
host/port-bearing timer text, and native elapsed text follows the context.
Returned Other/Protocol become Io on the specified native paths; existing
exhaustive enums and public signatures remain. Category can change from unknown
to native timeout/disconnected/unsupported/cancelled, local invalid_input or
malformed remote protocol. Opaque crypto/backend/panic stays unknown. Safe
correlations/Display prefixes change, while existing machine fields/statuses/
exit codes and success output remain.

Owned host fixtures cover original IO/opaque categories, bounded cyclic sources,
real async JoinError cancellation and blocking JoinError panic, actual PSK dial
refusal and closed-stream handshake failure, local in-memory credential loaders,
both Base64 extraction functions, received key-length conversion and the actual
RemotePairing listener request/response over loopback TCP. The listener fixture
tests malformed Base64, failed authentication and valid encrypted port success;
it joins its peer and drops the local control channel. Existing endpoint tests
now verify original address/Elapsed objects rather than only text. No worker
fault is injected into an entire discovery/device process, no actual blocking
task is claimed cancellable after it starts, and the two other dial timers are
source-reviewed rather than network timeout-injected. Not every listener stage
is independently fault-injected. No real credential directory, pairing/device
operation, trust-policy change or Apple request occurs.

Seven inner pairing adapters now use the same retained-cause context through
existing PairingError::Io: private-seed slice conversion, HKDF expansion, both
cipher key initializations, payload encryption/decryption and device-info OPACK
encoding. Seed/key/output length rejection is InvalidInput; received payload
authentication failure is InvalidData/protocol. Opaque encryption remains
unknown, and OPACK preserves its typed category. Explicit source inspection
reaches the original TryFromSliceError, hkdf::InvalidLength, cipher InvalidLength,
AEAD Error or OpackError through IO; wrapper Debug omits private payloads.
Nested credential loaders retain both the PairingError and its native cause.
The public manual-pairing transport now retains these four PairingError paths,
including M6 verification, through the outer contract below. This preserves
already-retained native chains; it does not reconstruct earlier legacy strings
or establish a successful full device pairing workflow.

Affected Crypto(String)/Protocol(String) failures now return Io, with changed
Display prefixes/correlation and more precise categories. Match class/source
rather than old variants or message text for these internal failures. PairingError
is inside a crate-private module; its three variants remain, as do external APIs.
The legacy Protocol variant has a targeted dead-code allowance after its sole
OPACK producer now retains Io, with other warnings still checked. HKDF enables
its existing dependency's std feature so InvalidLength implements standard Error; versions,
lockfile and service feature names/defaults/groups stay unchanged. Successful
derivation lengths, salts/info, nonces, AEAD, signing, wire fields and optional
M6 information handling are unchanged. No extra verification or trust behavior
is introduced. PairingError's classifier stays tunnel-gated because the pairing
module/type itself only exists with tunnel; default builds have no such type.
Ed25519 signing is infallible here, and SRP proof verification returns bool,
so neither supplies a discarded native signature-validation error.

Owned actual constructor, M6 authentication and device-info encoding faults
verify sources, safe Debug and no partial success. Fixed HKDF values independently
computed with Python hmac and authenticated M5 contents/signature check preserved
success. Native HKDF/key-length errors and bounded in-place AEAD errors pass
through the production context adapter; these do not inject failure into the
fixed 32-byte production HKDF/key initialization or Vec encryption call.

Failure does not authorize retry or prove remote cleanup. Dropping an awaiting
caller still does not stop already-running blocking work. Other legacy string
outcomes, CLI/HTTP/stream/binding boundaries and L01/L03/device conditions remain
pending; the new source adapters do not add invalidation or reuse rules.

## Manual-pairing transport sources

Sixteen adapters retain original RSD/bootstrap/codec XpcError, H2Error,
PairingError, header TryFromSliceError, AEAD Error and serde_json::Error objects
through existing PairingTransportError::Io and the bounded fixed-context
snapshot. Native IO kinds remain, H2 protocol maps to InvalidData, received
authentication/JSON/header faults are protocol, and opaque crypto/encryption
stays unknown. Named XPC service misses/closed states retain service_missing/
disconnected rather than becoming generic protocol text. Four inner crypto
stages retain PairingError and its own source, including the inner M6 native
authentication error. Pre-existing string-only fields remain string-only.

The service dial wrapper also retains original Tokio Elapsed under TimedOut
with fixed context instead of address-bearing text; the existing timeout value
and schedule remain. A short private header-length input now yields the native
slice error under InvalidData rather than panicking on indexing. The actual
H2 reader still requests 24 header bytes. Existing body-length limit diagnostics
remain PairingTransportError::Xpc: the shared checked-length helper currently
returns String, not a native Error. No cause is fabricated for it or for legacy
created failure/proof strings; those typed guard/lifecycle details remain work.

Affected Xpc/Rsd/Crypto failures migrate to existing Io. All nine exhaustive
public variants, root function signature/future and credential fields remain.
Class/source inspection replaces affected variant or Display parsing; safe
correlations/Display prefixes can change. Requests, sequence numbers/nonces,
salts/derivation, signing, stream selection/bootstrap, body limits, trust, retry
schedule, remote-error handling and optional/successful unlock-key results stay.
No dependency/lockfile/service-feature or binding/CLI-schema changes.

Owned H2 streams test actual closed write, header/body read, decoder and
bootstrap faults, and the actual unlock request/response with AEAD/JSON failures
and both returned key/optional-empty success. Peers bound reads, verify request
ID/sequence/plaintext, shutdown and join; the local framer is dropped. A real
RSD refusal uses an owned non-listening IPv6 socket; real Elapsed uses an expired
pending future. Typed adapters cover native kinds, named XPC and opaque sources,
and actual inner M6/seed/SRP errors. The root API future is externally type-checked
without polling. This does not inject every stage of the public full SRP flow,
the existing 15-second network dial timeout, huge encoder-size or Vec encryption failure;
the encryption adapter uses a typed native-error fixture. No device, new trust,
real credential directory or Apple request is involved. Local drop/join does not
prove remote cleanup, rollback, invalidation/reuse or safe retry.

## Default lockdown native sources

Default builds now retain request/response plist, ProductVersion semver,
certificate/key PEM, rustls client-auth, DNS-name and TLS-handshake errors through
existing LockdownError::Io. The common private source/context adapter is always
compiled; cached Unknown and the 32-source construction bound remain. Local
serialization/PEM/name failures are InvalidInput; received plist/version faults
are InvalidData/protocol. Opaque rustls configuration errors remain unknown.
TLS preserves original IO kind and backend payload. These categories express
typed causes or the explicit input/response role, without matching prose.

PairRecord::load/load_from_path now return existing PairRecordError::Read for
missing files and malformed local plists. Missing-file failures retain native
NotFound IO/OS code; malformed plists retain plist::Error under InvalidInput.
Both keep their previous `not_found`/`invalid_input` classes. The Read path field
and native sources remain explicitly inspectable; Display/Debug still omit
credential paths/data. All three variants of both public error enums remain.
Migrate affected NotFound/Parse/Protocol matches to class/source observations;
Display prefixes and safe correlations can change. Original source/Display
access remains private data, while context Debug omits payloads.

The explicit path remains authoritative and the identifier argument remains
accepted. The file bound/Zeroizing key and buffer, frame budget, ProductVersion
normalization, selectors and successful output remain. TLS provider/verifier,
server names and parameters stay; no timeout, retry or trust policy is added.
Created validation failures (empty certificate, missing key/fields, disabled SSL,
read-ahead guard) retain their existing Protocol outcomes without invented causes.

Owned host fixtures cover actual malformed/valid serializers and files, native
PEM/DER/name errors, closed TLS write and malformed peer/backend, successful
duplex TLS/application IO, product-version normalization and root API consumers.
The internally constructed version client and unpolled root constructor do not
prove full lockdown session/device pairing. Shared-context default tests cover
native kinds/Unknown, bounded cyclic sources and actual Join flags. Native
classification does not authorize retry or certify running-worker cancellation,
remote cleanup or stream reuse. Supervised OpenSSL/record-save, other codec/
created outcome/stream/binding and L01/L03/A03 conditions remain. Evidence:
[default lockdown source validation](validation/default-lockdown-sources-2026-10-02.md).

## Supervised pairing native sources

With supervised-pair, 51 OpenSSL adapters, record plist serialization and file
writing retain their original native sources through existing LockdownError::Io
and bounded fixed context. Opaque backend/key/certificate/signing stacks stay
unknown, local PKCS12 parsing/unlocking is invalid_input, malformed received
device public keys are protocol, and file errors preserve native IO kinds.
Original ErrorStack codes/entries survive. No permission or unsupported class
is inferred from backend text. Missing certificate/key and created device
response validation retain their existing Protocol outcomes.

An interior NUL password now returns InvalidInput with the original NulError
instead of panicking inside the dependency's CString unwrap. Validation follows
DER parsing and precedes parse2; valid passwords keep their existing path. The
NulError object owns original password bytes, so source/Debug access is sensitive.
Wrapper Debug omits these bytes; raw Rust Display/source access remains private
data. Affected Protocol-to-Io, category/Display/correlation migrations are
explicit; the exhaustive error enum, root functions and FullPairRecord fields
stay. Consumers should use class/source rather than old message/variant parsing.

Certificate generation parameters, PKCS7 BINARY, Pair request fields/order,
private-key stripping, results, file format and owner-only publication/replacement
policy remain. No timeout, retry, trust or lifecycle policy is added. Owned
fixtures verify actual P12/NUL/public-key/backend/signing/file failures and a
complete root exchange with generated certificate/signature checks and signed
binary challenge. Test-only PKCS7 NOVERIFY/NOINTERN uses a known synthetic signer
and verifies the signature without establishing chain trust. No physical
supervised device/enrollment/Trust or remote cleanup/reuse is certified; not
every backend or save stage is fault-injected. MCInstall's analogous P12 boundary
has subsequent coverage below; other sources/streams/bindings and L01/L03/A03
remain. Evidence:
[supervised pairing source validation](validation/supervised-pairing-sources-2026-10-02.md).

## MCInstall native sources and explicit P12 outcomes

Five OpenSSL adapters retain original ErrorStack objects under existing
McInstallError::Io and bounded private fixed context. Local PKCS12 DER/password
failures are invalid_input; certificate DER, signing-stack allocation and PKCS7
signing/encoding failures remain unknown. The remove-profile identifier encoder
propagates original plist::Error as existing Plist instead of Protocol(String).
Its class remains protocol. No backend text is used for classification.

Interior NUL passwords return invalid_input with the original NulError after
DER parsing and before the dependency's parse2 unwrap. This native object owns
password bytes; explicit source/Debug access is private data. Context Debug
omits payloads. Missing parsed certificate/key predicates return InvalidInput
IO without inventing an underlying backend cause. Without supervised-pair,
both public P12 entry points retain an explicit CoreError::Unsupported feature
decision through Io and perform no writes. Legacy manually constructed Crypto
remains unknown; PasscodeSet remains precondition.

All five public error variants, root exports/signatures and feature definitions
stay. Affected Crypto-to-Io and Protocol-to-Plist matches should migrate to
class/source observations; Display prefixes, correlations and categories can
change. Existing PKCS7 BINARY, request keys/order, challenge handling, zeroization,
ACK gates and best-effort profile-list fallback remain. No timeout, retry, trust
or lifecycle policy is added. Owned fixtures cover actual faults, each ACK gate
and a complete public silent-install exchange with known-signer verification;
test-only NOVERIFY/NOINTERN does not establish chain trust. Physical supervision,
profile installation, remote cleanup/reuse, every backend/allocator/encoder stage
and memory-lifecycle guarantees remain unverified. Existing profile metadata
integer conversion and other codec/guard/stream/binding work remain. Evidence:
[MCInstall source validation](validation/mcinstall-sources-2026-10-02.md).

## Deactivation fallback and activation HTTP sources

`ConnectedDevice::deactivate` attempts mobileactivationd and, on failure, one
legacy lockdown fallback as before. Both failed attempts now return existing
CoreError::Io with additive private-field `DeactivationFallbackError`, exported
at root and device with feature mobileactivation. Borrow the observation through
the IO payload's `get_ref().downcast_ref::<DeactivationFallbackError>()`.
`service_error()` and `fallback_error()` retain their original CoreError objects;
`service_class()` and `fallback_class()` are bounded classifications taken before
wrapping. Its standard source is the final fallback error. The final category
controls classification even when unknown; an earlier precise failure cannot
override it or turn opaque Other text into a guessed category. Safe Debug shows
categories, not payloads. Explicit Display/source access can expose private data.

Modern service errors use the existing source-preserving activation adapter.
Combined failure previously became Other(String); a failure establishing the
fallback connection previously dropped the earlier cause. Both now retain the
two observations. Primary success still skips fallback construction, successful
fallback still returns success without an earlier-failure observation, and no
additional retry is introduced. Returned variants/Display and safe correlation
can change. No success or category certifies rollback, stream reuse or cleanup.

ActivationHttpClient preserves original reqwest errors from build/send/chunk
through private fixed-phase IO payloads. Native timeout/builder flags and typed
underlying sources participate in classification; otherwise the IO fallback
applies. Endpoint parse errors keep their original source; HTTPS, credentials
and missing custom-endpoint opt-in remain typed local InvalidInput rejections.
These seven former Http-string paths migrate to MobileActivationError::Io:
native request timeout changes service_failure to timeout, endpoint rejection
changes service_failure to invalid_input, and other HTTP causes use their typed
chain. Status and request/response body-budget rejections remain Http/service_failure.
Client Debug hashes both URLs; original Rust cause access can reveal private
queries. Existing HTTPS verification, exact official endpoints, explicit custom
opt-in, redirect policy, timeout values, body limits, wire headers and success
response fields remain unchanged.

Owned loopback fixtures cover refusal, stalled TLS handshake, verified HTTPS
body truncation/timeout, complete response/status rejection, and rejection of
an untrusted test certificate. Positive TLS fixtures install an ephemeral root
only in the private test client, with verification enabled; they do not add a
production custom-CA interface. Interrupted response reads return no successful
ActivationHttpResponse; a request may already have been sent and remote effects
remain unknown. Partial read-buffer zeroization, borrowed-client invalidation,
ordered cleanup and real Apple/device acceptance remain separate A02/L01/L03
conditions. The production CLI renderer retains exit 1, activation phase and
unknown effects, and hides private fixture values. Host tests do not execute
paired ConnectedDevice routing, TLS lockdown setup or physical deactivation.

## Symbols, activation and pasteboard sources

The following adapters preserve original typed causes instead of formatting
them into strings. Public exhaustive enums and function signatures remain
unchanged; affected returned variants, Display prefixes and safe correlations
can change. Consumers should use typed causes or classify_error instead of
matching the former flattened variant or diagnostic words.

| Boundary | Returned migration / retained source |
| --- | --- |
| RemoteFetchSymbolsClient H2 connect/bootstrap/catalog/file operations and XPC encoding/buffering/decoding | FetchSymbolsError::Protocol(String) becomes existing Io, with fixed phase and original H2Error/XpcError; native IO retains kind/class, genuine protocol failures stay protocol |
| CLI remote symbols connection | Opaque anyhow string becomes original FetchSymbolsError chain under fixed connect remote symbols context |
| ConnectedDevice modern activation record application | CoreError::Other(String) becomes existing Io with fixed mobile activation context and original MobileActivationError; native kinds and typed Timeout/Http/Protocol/Plist causes survive |
| Pasteboard notification snapshot connection | PasteboardError::Protocol(String) becomes existing Xpc/Io with fixed connect pasteboard XPC context and original CoreError; precise native, named service-missing and explicit unsupported sources remain distinct |

Modern activation's legacy-wrapper routing, service command and later lockdown
acknowledgement are unchanged. The CLI passes its original CoreError through the
existing absolute-deadline wrapper. Actual expiry now retains Tokio Elapsed;
observed Ctrl-C failure retains OperationCancelled, while signal-registration
failure keeps native IO under context. Timeout/cancellation Display summaries
are retained; registration-error context and safe correlations can change.
Plain Interrupted is still IO. Deadline/signal selection and durations remain
unchanged; caller-observed cancellation and local future drop do not certify
rollback, Apple cleanup or safe retry. Deactivation's separate combined fallback
and activation HTTP/source adapters remain pending.

CLI symbols, activation and pasteboard keep success output, command/flag shapes
and final error envelope fields. Retained sources can change formerly unknown
or protocol categories to timeout, cancelled, disconnected, permission_denied,
service_missing or explicit unsupported. Final errors still use exit 1 and
unknown effects. A host-output/native category does not identify device state.
Private native payloads remain accessible to explicit Rust cause inspection;
owner/new wrapper Debug and safe CLI JSON keep them hidden.

A failed symbols download can leave completed local chunks; a later incomplete
H2 read does not deliver its partial bytes to the sink. No success size/result
is returned after failure. Discard the failed remote client; these changes do
not automatically invalidate, close/join or certify reuse of its borrowed stream.
The bounded host fixture retains a complete 10 MiB chunk before a second-chunk
EOF, and a successful limited download retains exact requested bytes. H2 frame,
buffer and file budgets are unchanged. Pasteboard registration can precede a
baseline connection error; the host fixture observes one baseline attempt,
no monitor result, registration bytes and local notification-stream EOF.
Neither fixture is device cleanup acknowledgement.

Host evidence covers real public remote-symbols transport/codec faults, a real
modern service write fault through the shared CoreError mapper, typed mapping
fixtures, the production CLI deadline/signal helper over controlled futures,
the actual pasteboard monitor baseline with the shared connection/snapshot path,
and actual safe error rendering. It does not establish paired-TLS ConnectedDevice
activation/acknowledgement, actual OS Ctrl-C registration/delivery, remote-symbol
device workflows or physical pasteboard state. Other source/JSON/callback/binding,
L01/L03/A03 and platform/device conditions remain pending.

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
