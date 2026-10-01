# Error and machine-output contract

A02 is in progress. Its first additive Rust classifier is implemented; the
roadmap records this slice's validation and keeps CLI/HTTP/C/Python mappings, stream
semantics and upstream information-loss fixes pending. R04's accepted
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

Current CLI success formats and failure exits remain as recorded by R04. The
new Rust classifier alone does not add machine error fields to CLI stderr,
HTTP, Python exceptions or C status values. Those adapters are pending A02
work and must preserve their existing callers through additive or opt-in forms.
Existing safe diagnostic messages remain useful for correlation but must not
be parsed as stable natural-language categories.

Stream framing, ordinary stderr progress/callback failure, partial-operation
status, and JSON commands that currently emit human status text remain under
review. A partial stream or an output error can follow completed operations;
no rollback, automatic retry or device cleanup acknowledgement is promised.
L01 retains the stronger lifecycle/deadline acceptance.
