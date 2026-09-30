# Diagnostic data contract

Device identifiers, credentials and device content have distinct output purposes.
Automatic diagnostics use a fixed operation phase, typed error category, I/O
kind when available, and opaque process-local correlation values. Increasing
CLI verbosity does not opt in to raw credentials, protocol error bodies or
error source chains. Rust callers can still inspect the original typed error
and its sources when they explicitly need them.

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
| WebDriver failure | Existing status and `value.error`, fixed phase plus safe `value.message` | Successful script/element results remain debugger data |
| Device CDP protocol failure | Request/session routing and numeric error code retained; message replaced; arbitrary error data omitted | Runtime results, exceptionDetails, console events and page data retain their protocol purpose |
| Python operation exception | Existing RuntimeError/ValueError type, fixed phase or allowed-value hint | `list_devices`, attributes and `connect_info` retain usable selector data |
| Python tunnel/patch repr | Address pseudonym, ports and counts | `connect_info` exposes the actual address for connection setup |
| C failure | Existing numeric status and output reset/ownership rules | Successful selector JSON remains API data |
| dproxy progress | Protocol category, direction and summary correlation | Explicit traffic recording files still contain raw/decoded device traffic |

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

Local HTTP/WebSocket fixtures test the production manager, WebDriver and CDP
failure-formatting functions with synthetic source chains/device error data.
They are host adapter evidence, not simulated complete WebInspector workflows
or physical-device evidence. The full validator also checks Python exceptions
and repr, installed Python argument errors and captured C numeric failures.

S04 remains open for the remaining output-site and derived-Debug audit, framework
extractor rejection replies, live timeout/cleanup log capture and device
observations. Authentication, authorization, Host/Origin and protected federation
transport remain S01–S03; safe error text does not supply those controls.
