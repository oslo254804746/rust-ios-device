# Error and machine-output contract

A02 is in progress. Additive Rust classification and opt-in final CLI machine
errors are implemented; the roadmap keeps HTTP/C/Python mappings, stream semantics,
probe observations and remaining information-loss fixes pending. R04's accepted
[stdout/encoding migration](plist-json-contract.md) remains independent.

## Rust classification

`ios_core::{classify_error, ErrorClass}` and `CoreError::class()` inspect typed
errors and their original sources. Existing public error variants, fields,
Display and source chains remain unchanged, including exhaustive CoreError
matches. ErrorClass is a new non-exhaustive enum, serializes to a snake_case
string, and exposes the same stable code through `as_str()`. Callers must keep
a fallback for future categories. Human diagnostic text is not a classifier.

| Stable code | Typed evidence |
| --- | --- |
| `timeout` | IO TimedOut, Tokio Elapsed, or an existing service Timeout variant |
| `cancelled` | A cancelled Tokio JoinError; it does not acknowledge device cleanup |
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

An unattempted versus failed probe still needs an owner-maintained observation.
`ConnectedDevice::rsd() == None` currently includes skipped and failed discovery;
it cannot truthfully be relabelled as an unattempted probe. An empty feature list
retains its documented permissive legacy behavior; it is not a deny-all list.

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
exist. No new machine fields are added to existing HTTP, Python exceptions or
C statuses; those adapters remain pending A02 work with additive contracts.

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
