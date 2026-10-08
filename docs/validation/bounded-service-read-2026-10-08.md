# Caller-bounded AFC reads and InstallationProxy Browse

Accepted on 2026-10-08, Linux x86_64. Core implementation:
`655733c` (`feat(core): add caller bounded AFC reads and app Browse budgets`).
This completes two native prerequisites for A03. Python/C SDK ownership,
bindings, asynchronous cancellation and device workflows remain unfinished.

## Delivered behavior

`AfcClient::read_file_with_limit(path, max_bytes)` returns a complete file or an
error; it never returns a truncated success. The effective limit is the smaller
of the caller's limit and the existing 256 MiB ceiling. Zero permits only an
empty file. Reads request at most the remaining allowance plus one byte, capped
at 64 KiB, so the method does not depend on remote file-size metadata. Open,
read and close replies admit declared lengths before converting to usize or
allocating their bodies. A caller-limit refusal classifies as `resource_limit`
through the existing `AfcError::Io` variant.

An undrained or interrupted frame invalidates the client. A fully consumed
limit refusal can retain reuse after successful file close. Returned failures
attempt close with the existing independent five-second cleanup budget; both
primary and cleanup errors remain owned. The optional request timeout remains
per request, not a whole-file deadline. Dropping an operation after open cannot
await close and invalidates ownership; drop the client to release its transport.
Legacy signatures, wire and public error variants are retained.

`InstallationProxy::list_user_apps_with_limits(BrowseLimits)` shares one checked
total across request send/flush, every reply and final parsing. Defaults admit
10,000 entries, 16 MiB cumulative framed bytes and 1,000 response frames. Builders
cannot raise the existing hard ceilings of 1,000 frames, one million entries or
4 MiB per frame. Cumulative declared wire lengths are checked before body
allocation. Zero or unrepresentable totals fail before I/O; entry, frame and
wire refusals retain typed resource categories through existing error variants.

The checked parser requires correctly typed Status, CurrentList and entries;
missing or malformed metadata cannot become an empty successful list. A
complete explicit Complete reply carrying a remote business error can preserve
reuse. Timeout, cancellation, parse or limit failure after commitment makes the
client unusable. Subsequent calls through every legacy entry reject locally
before transport I/O. `is_usable()` reports this local framing state. Legacy
Browse timing remains unchanged unless the caller selects the new method.
An error acquired while polling across the total retains its original source;
late successful completion is suppressed.

These are logical file/output/wire bounds. They do not establish allocator
capacity, peak RSS, decoder CPU, arbitrary input-path bounds or safe replay of
device mutations.

## Validation

The main agent integrated independent static deliveries, formatted and ran Rust
jobs serially with source hashes frozen. Agents did not compile.

- Fourteen new AFC regressions use actual protocol frames, duplex and owned TCP
  peers: exact/zero/over limits, hostile declared lengths including u64::MAX,
  EOF, original elapsed errors, dual cleanup failures, cancellation, late replies
  and peer release. Minimal `afc` feature tests passed.
- Sixteen new Browse regressions cover total send/read/parse timing, cumulative
  budgets, malformed/missing metadata, every legacy reuse entrance, native error
  identity, cancellation and fresh-session TCP EOF. Minimal `apps` feature tests
  passed.
- Affected all-feature Core tests passed: 1,538 unit tests, 55 public-contract
  tests and the remaining integration/doctests; zero failures.
- The final combined full run at this Core acceptance boundary exited zero:
  2,883 Rust and 13 Python host tests passed, zero failures, two existing ignored.
  Format, workspace check, strict Clippy, MSRV 1.80.1, 66 individual feature
  checks/four groups, docs/examples, C ABI, rebuilt Python wheel/sdist contracts,
  73 tooling tests, inventory, raw diagnostics and host/CLI/backup contracts passed.

That full run included the pending manager reconnect proposal. It certifies the
Core code above; the manager proposal has a separate review and correction
boundary and is not accepted by this document. The exact combined source is
retained in the frozen patch, rather than represented as a clean checkout of
`655733c`. The full run checked 481 product-file hashes unchanged. Earlier fixture
type errors and Clippy/redaction guard failures were fixed without lowering
assertions; failed logs remain archived.

Evidence directory:
`target/validation/manager-reconnect-sdk-primitives-2026-10-08/`.
Final Core-acceptance run: `full-final-result.json`, 473.651 seconds, exit zero.
Log SHA256: `98874851123e69ef2e37e3cf57ad61132b21f7e9802289d58508904916b229d5`.
Frozen implementation patch SHA256:
`a5e836e10cc773ed453888675343a6649ab9fbcbfc3e1085ced05ca07b5143d7`.

## Migration and remaining acceptance

Select the additive methods when a caller needs file/output or total-operation
limits. After an interrupted checked Browse, discard the client; older methods
now also reject that client's retained late reply. Check the original operation
error and any AFC cleanup error before deciding how to recover. No automatic
installation, erase or other mutation retry is introduced.

Twenty-two device rows remain NOT_RUN. Selected device/iOS/USB/Wi-Fi state,
representative SDK workflows and macOS/Windows evidence remain missing. A03,
L01 and all other original main-task requirements stay open; capability maturity
is unchanged. Q04 and all its children remain excluded.
