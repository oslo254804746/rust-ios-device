# Diagnostic data contract

Device identifiers, credentials and device content have distinct output purposes.
Automatic diagnostics use a fixed operation phase, typed error category, I/O
kind when available, and opaque process-local correlation values. Increasing
CLI verbosity does not opt in to raw credentials, protocol error bodies or
error source chains. Rust callers can still inspect the original typed error
and its sources when they explicitly need them.

The CLI subscriber accepts only the reviewed `ios`, `ios_cli` and `ios_core`
target namespaces at the selected level. Namespace component boundaries are
checked; similarly named targets are excluded. Dependency logs remain excluded
at every verbosity, including TRACE, because their handshake headers, frame
payloads, close reasons, URLs and IO error text have independent formatting
policies. The project emits its own safe phase/category/correlation diagnostics
for failures instead. Library/SDK embedders control their own subscribers and
must apply an equivalent dependency policy; the library does not install a
global subscriber for another application.

`ios_core::redaction::diagnostic_id` pseudonymizes identifiers, addresses, paths
and untrusted names. `diagnostic_error` formats errors without their text or
Debug payloads. Correlation uses a random process key; it cannot correlate
separate runs. Entropy failure produces a fixed redacted label. Error hashing
consumes at most 4096 formatted bytes and eight sources, including cyclic
source chains, without assembling an intermediate error string. These limits
bound hashing, not the behavior of an arbitrary caller-written Rust Display
implementation.

| Surface | Diagnostic behavior | Explicit data output |
| --- | --- | --- |
| CLI command failure | Canonical command phase, category and correlation; runtime exit 1 | Command results keep their documented shape |
| CLI argument failure | Parser error kind, correlation and help hint; exit 2; invalid values omitted | Help/version retain normal output; credential environment values are hidden |
| Destructive-operation guard | Fixed `--force` confirmation hint through error contexts | No device operation starts merely to report this guard |
| Manager operation failure | HTTP status, fixed `message` phase, safe `error` category/correlation | Successful device/tunnel selectors keep their original identifiers |
| Manager missing/unsupported target | `device` pseudonym replaces diagnostic `udid`; no echoed IP | This is an experimental error-schema change, not a selector substitution |
| Framework HTTP rejection | Status and Allow retained; fixed HTTP phase, status category and request correlation; HEAD body empty | Query/JSON/path/handshake rejection bodies are discarded without reading them |
| WebDriver failure | Existing status and `value.error`, fixed phase plus safe `value.message` | Successful script/element results remain debugger data |
| Device CDP protocol failure | Request/session routing and numeric error code retained; message replaced; arbitrary error data omitted | Runtime results, exceptionDetails, console events and page data retain their protocol purpose |
| Python operation exception | Existing RuntimeError/ValueError type, fixed phase or allowed-value hint | `list_devices`, attributes and `connect_info` retain usable selector data |
| Python tunnel/patch repr | Address pseudonym, ports and counts | `connect_info` exposes the actual address for connection setup |
| C failure | Existing numeric status and output reset/ownership rules | Successful selector JSON remains API data |
| dproxy progress | Protocol category, direction and summary correlation | Explicit traffic recording files still contain raw/decoded device traffic |
| Rust owned error Debug | Type name, safe category and correlation; normal and pretty Debug omit fields/source text | Typed variants, Display and sources remain available to explicit API callers |
| Clipboard promise failure | Fixed `promise failed` phase and correlation in human/JSON/event output | Requested clipboard values remain data; the Rust snapshot retains its original typed payload |

Manager, CDP and Selenium apply the same HTTP fallback to errors produced before
a handler runs. Reviewed diagnostic helpers mark safe replies in server-only
response extensions; a client header cannot bypass the fallback. Unreviewed
error bodies and headers are not copied, apart from the framework's Allow
header. The fallback sets Cache-Control: no-store and retains HTTP status.

FullPairRecord, SupervisionIdentity and VerifyPairSession Debug omit credential
material, including certificates, key bytes and handshake/session keys.
TunnelInfo Debug pseudonymizes addresses while retaining numeric port/MTU
metadata, including builds without the tunnel feature. Value models such as
XpcValue and clipboard snapshots retain Debug for explicit data inspection;
automatic diagnostics must not format those payloads. This distinction also
applies to serialization: a credentials serializer is not a log formatter.

Device lists, explicitly requested syslog/packet captures, clipboard contents,
script values, exceptions and raw protocol recordings may contain personal or
secret data. Their successful output is not a diagnostic and is not silently
rewritten. The recording destination, permissions and sharing policy remain the
caller's responsibility under the documented output-root trust boundary. The
S04 changes do not certify every existing recording artifact as private.

Host checks run with `python scripts/validate.py diagnostics`. They launch the
actual CLI with synthetic credentials/identifiers for argument errors, host
validation, a usbmux address failure and credential-bearing help. They capture
stdout/stderr in bounded RAM (256 KiB total), check the original bytes first,
then independently check a masked copy. Reports contain case names, byte/hit
counts and outcomes, never raw output, argv or environment. Timeout, overflow,
incomplete capture, empty output and wrong exit/phase contracts fail the check.
Tool tests include UTF-8, hexadecimal, base64, URL and JSON representations,
output flooding and timeout cleanup. Unix captures own a new process session;
Windows host cases launch only the CLI, and general descendant supervision
remains part of Q02's unfinished workflow harness.

Credential preparation checks additionally create only owned temporary private
policy/token files, read the generated token with a fixed bound, and capture
`tunnel auth create`, an allowed check and a denied port at maximum verbosity.
They remove the default device selector and keep usbmux unavailable, so host
preparation must succeed without a device. The generated token and selector are
checked in original output and an independent masked copy; neither enters the
report. Preparing these files is not server-admission evidence.

The same raw-output gate now runs the production manager on an owned loopback
port with only list/server grants. Its monitor skips device discovery. It checks
unauthenticated/invalid-token/scope denials, allowed lists, malformed-policy
reload preserving admission, successful reload, actual private-token CLI clients
and shutdown. Original server stdout/stderr must contain the listening phase
without fixture token/selectors/policy/body data; timeout, truncation, incomplete
capture or a wrong HTTP/CLI contract fails. It persists only sanitized counters.
Four additional maximum-verbosity CLI captures now reject nonloopback forward,
WDA proxy, dproxy and raw screenshot-stream options before device/service access;
their runtime error phase and original secret variants are checked in the gate.
Doctor captures also exercise a legal private manager token and a missing file.
With usbmux intentionally unavailable, both overall exits are 1; acceptance
checks the separate manager entry (ok/failed) and retained host scope, so one
healthy probe cannot hide the independent failed host checks.

CDP/Selenium captures now reject implicit/remote plaintext trust at startup and
run each production listener for authorized adapter metadata and policy reload,
plus unauthenticated/expired/role refusals. No device method is admitted. The
owned processes end by controlled termination, with that platform's explicit
exit contract; this is not a graceful device teardown or a capture timeout.
Original headers, token, selector and malformed policy/body stay out of server
output and reports. That debug gate covered 32 CLI/service cases; current HTTP
boundary acceptance covers 37, including invalid config and missing debug
selectors. Three production listeners refuse original Host/Origin/proxy inputs
without emitting them before reload/device operations. Legacy manager GET is
read-only, shutdown is POST; omitted debug UDID no longer starts discovery.

Local HTTP/WebSocket fixtures test the production manager, WebDriver and CDP
failure-formatting functions with synthetic source chains/device error data.
They are host adapter evidence, not simulated complete WebInspector workflows
or physical-device evidence. The full validator also checks Python exceptions
and repr, installed Python argument errors and captured C numeric failures.

Additional host fixtures send invalid JSON/query/path values through real HTTP
servers and capture production userspace proxy timeout/EOF and backup rollback
failure logs in bounded RAM. Each requires the expected phase and rejects empty
capture. The async proxy fixture runs in an owned, time-limited child harness
with a global subscriber, isolating process-wide tracing filters from parallel
tests; child stdout/stderr are discarded and raw captured fields are never
persisted. A positive control confirms that the capture detects an actual raw
secret. These fixtures do not supply physical-device observations.

A real tokio-tungstenite loopback fixture also sends a synthetic UDID in the
request URL, a marked-sensitive Authorization header, text payloads and a close
reason. In a separate unfiltered child it requires actual request/message/close
logs and detects the original secret, proving why HeaderValue sensitivity alone
does not protect serialized handshake logs. With the production CLI subscriber,
the same protocol exchange succeeds, reviewed project phases remain captured
and raw dependency/secret output is absent. Both child streams are discarded;
capture is bounded RAM and the parent owns their time limit and termination.

S04 evidence remains scoped to the reviewed host surfaces; future authentication
and protocol changes must extend raw-output checks. Manager authentication and
device/port and fixed-device debug authorization are integrated; platform/device
acceptance, browser tickets/bootstrap and protected federation keep S01–S03 in progress. Safe
error text does not supply those controls.
