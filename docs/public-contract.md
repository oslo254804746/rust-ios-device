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
