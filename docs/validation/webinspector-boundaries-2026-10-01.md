# WebInspector module and injection acceptance

Date: 2026-10-01 UTC. Host: Linux x86_64, Rust 1.97.1, MSRV alias 1.80.1.
This records R02 host module/compatibility acceptance. Device and external CDP
client interoperability remain separate F03/F04 and capability conditions.

Implementation: `07ec8977fd5e8e421982348bcc79e376909d698e`. The complete gate
passed on this reviewed implementation before its focused local commit.

## Scope and contracts

The core facade retains its public models, client/session types, errors,
constants and `cdp_browser`. Six private modules separate model parsing,
bounded plist framing, deadlines, stream/cache/event state, InspectorSession
Target state and AutomationSession. Private cache/session fields remain private;
only the two methods used across core owners gain internal visibility.

The CLI facade retains `WebInspectorCmd`. Grammar/dispatch, reusable workflows,
one-shot tasks, shell input, safe diagnostics, device connection strategy,
HTTP state/startup, CDP and WebDriver each have explicit private owners. Existing
credential/session leases and aborting browser page task guards remain in place.
Tests move with owners instead of widening private runtime fields.

`InspectorSource` returns an owned byte stream and optional device lifetime guard.
Production installs `DeviceInspectorSource` exclusively, preserving the existing
iOS-version route probe, total connection timeout and real guard lifetime. Tests
supply owned duplex streams without forging `ConnectedDevice`. No command,
environment variable or HTTP argument switches production to a mock source.
CDP page/browser and Selenium connection calls remain after admission.

No user-visible parameter, endpoint or result migration is introduced. Review
checks preserve all command grammar, 24 production route paths and the exact
`OpenedTabRow`/`LaunchResult` fields. Existing diagnostic shapes, JS exception
results and requested Runtime values retain their distinction. General stdout
failure policy and richer error categories remain R04/A02 work.

The 111 original core function bodies match after whitespace normalization. All
118 original CLI functions remain; 108 bodies match and ten changes were reviewed
as source/state/internal-module calls. This is a review aid, not an AST proof.
Existing Apple selectors, JavaScript command bytes, framing and budgets remain.

## Narrow checks

Commands use `--locked`; Rust/Python splitting follows roadmap section 11.

```sh
cargo test -p ios-core --all-features --locked webinspector
cargo test -p ios-core --no-default-features --features webinspector --locked webinspector
cargo clippy -p ios-core --all-targets --all-features --locked -- -D warnings
cargo test -p ios-cli --locked 'cmd::webinspector::'
cargo clippy -p ios-cli --all-targets --all-features --locked -- -D warnings
cargo test -p ios-core --all-features --locked --test webinspector --test public_contract
cargo test -p ios-core --no-default-features --features webinspector --locked --test webinspector --test public_contract
```

Results: core unit filter 20 PASS plus one filtered integration PASS; the entire
WebInspector integration target 12 PASS, public consumer seven PASS with all
features and six PASS with minimal WebInspector. CLI 31 PASS: 28 retained cases
and three new bridge tests. Both core/CLI strict Clippy pass.

The new tests run the actual protected Axum routers on owned loopback listeners:

- Unauthenticated CDP discovery and WebDriver session creation make zero source
  calls. Authorized source failure reaches the exact selected synthetic device
  and returns HTTP 500 with a typed, redacted diagnostic; no empty success.
- CDP `/json/list` returns the synthetic qualified page and actual listener URL.
  A real page WebSocket traverses plist handshake/listing, unpaused socket setup,
  Target envelopes and nested Runtime responses. A synthetic protocol failure
  preserves the request ID/code while hiding its message/data. Closing sends
  `_rpc_forwardDidClose:` and releases the owned stream.
- A real browser WebSocket performs target discovery, flattened attachment,
  Runtime evaluation with the outer session ID, explicit detach and teardown.
  Joined service tasks and derived counters check request flow and cleanup.

The fixture reuses protocol shapes already covered in `tests/webinspector.rs`.
`Runtime.syntheticFailure` exists solely in tests. Intermediate failures included
missing imports/derive ownership, fixture dictionary inference and Drop-type
struct update errors, then an incorrect fixture teardown selector (29 PASS/two
FAIL). Correcting the fixture to the existing core selector gave 31 PASS. The
mistyped `--test services` command failed before execution; the correct complete
target checks above passed. Full history remains in the roadmap delivery record.

## Complete gate

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

`/tmp/ios-r02-full.log`: PASS, including section 11 formatting, all-target/all-feature
compile and strict Clippy, split Rust 1670 PASS/zero FAIL/two existing ignored,
Python eight PASS and MSRV. The ignored cases remain the existing PBKDF2 stress
case and separately gated S02 Chromium probe. All 66 minimal feature cases and
four groups pass, as do default/full doctests and downstream examples, C ABI
header/link/error/ownership capture, rebuilt wheel and sdist installs/imports
on Python 3.12.13, 38 tooling tests, 118 capability/help records, 38 raw diagnostic
cases, five host regression cases, four CLI list contracts and three local backup
contracts. The 22 device regression cases remain NOT_RUN. Earlier Python
3.9/3.13 artifact and S02 browser evidence is retained, not rerun here.

The accepted `ios` 0.1.13 binary SHA-256 is
`b97a99409fe6a4d9fa598cb9d3064bbd46d3d3effb8f31b1644d515c216845c2`.
No capture contains a raw sensitive match or masks one to manufacture a pass.
Diff whitespace and 55 changed-document local links pass.

## Remaining evidence

External CDP clients through this injected Rust bridge are F03 acceptance work.
Physical Safari/WebView/JSContext, automation permissions, iOS-version routes,
device cancellation/teardown and platform execution require their own evidence.
These host tests do not upgrade capability maturity or promise stronger L01
deadlines/cancellation behavior.
