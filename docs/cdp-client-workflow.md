# Actual CDP client fixture

This tool runs pinned actual clients against the production Rust bridge and an
owned synthetic WebInspector JSContext. Host acceptance: Puppeteer25.12.0 completes
the Runtime workflow; Playwright1.63.0 fails initialization at
Browser.setDownloadBehavior with -32601. The expected-rejection test passes without
claiming Playwright workflow support. [Evidence](validation/cdp-client-workflow-2026-10-09.md).

## Reproduce

Use Node>=22.12.0 (verified24.19.0) and npm. No device or browser download is needed.
From the repository root, install the locked libraries and run the three explicit
gates as one serial Rust task:

```sh
npm ci --prefix scripts/cdp-clients --ignore-scripts --no-audit --no-fund
CARGO_INCREMENTAL=0 cargo test -p ios-cli --all-features --locked cmd::webinspector::bridge_tests::client_tests:: -- --ignored --nocapture --test-threads=1
```

IOS_CDP_CLIENT_NODE can select an absolute Node binary. Ordinary cargo test skips
these dependency-requiring gates; it does not establish client acceptance.
The script uses the real client APIs, public Puppeteer logger and actual Playwright
protocol diagnostics. No standalone fake CDP endpoint or forged replies are used.
Each gate owns its temporary policy, loopback listener, original native peers,
Node process and bounded pipes. Cancellation waits for actual child wait/pipe
joins and native EOF before asserting results. Failed captures preserve safe
bounded evidence; Node/library failures cannot masquerade as a download rejection.

The safe machine report distinguishes runtime-workflow from
initialization-rejection. fulfilled applies to the requested fixture expectation,
not general client compatibility. Each correlated -32601 remains visible.
The bridge's default browserContextId is a local logical target namespace,
not an Apple storage/isolation identifier. Browser-context creation remains rejected.

## Download implementation boundary

CDP [Browser.setDownloadBehavior](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Browser.pdl)
requires actual policy control and, for allow modes, file destination handling.
The inspected WebKit [Page](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/JavaScriptCore/inspector/protocol/Page.json)
and [Target](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/JavaScriptCore/inspector/protocol/Target.json)
schemas do not establish a verified translation for this bridge. Schema inspection
on2026-10-09 is not runtime evidence for an iOS release. Current behavior is -32601.

A real implementation must first capture the chosen Safari/WebView service's
begin/progress/complete/cancel protocol and prove which connection owns download
policy. Treat another automation service as a separate owned route if required;
do not send guessed automation commands through WebInspector. Retain original
request/event IDs and check browser-context/target/frame correlation.

Only then introduce an explicit scoped download controller: admit an authorized
host output root and byte/concurrency/total/cleanup budgets; transfer actual bytes
into owned temporary files; publish final files only after confirmed completion.
Cancellation must stop the original source and preserve primary/cleanup failures.
A deny result must demonstrate the engine prevented a download. Allow/default
modes must prove their actual effects and reset behavior; unsupported modes retain
explicit errors. Returning an empty success cannot supply these guarantees.

Use this actual-client fixture for request/event/file checks, abort during transfer,
late completion, duplicate events, failures and policy reset, then perform the
representative device workflows required by F04. Missing evidence is the selected
device/service/version, policy-control and byte-transfer mapping. Page/iframe,
console/context destruction, input, Fetch and metrics remain separate F04 work.
