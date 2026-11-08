# Actual CDP client fixture

This tool runs pinned actual clients against the production Rust bridge and an
owned synthetic WebInspector JSContext. Host and physical-device acceptance cover
Puppeteer 25.12.0, Playwright 1.63.0 (`connectOverCDP`), and Selenium 4.35.0;
isolated synthetic JSContext fixtures also retain explicit negative tests where
target-specific capabilities are omitted. See the [capability inventory](capabilities.md).

## Reproduce

Use Node>=22.12.0 (verified 24.19.0) and npm. No device or browser download is needed.
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
schemas do not establish a verified byte-transfer translation for arbitrary
JSContext targets. Controlled `Page.setDownloadBehavior` and
`Browser.setDownloadBehavior` policy handling is supported on page sessions for
client initialization (`connectOverCDP`).

A full byte-transfer download implementation must capture the chosen
Safari/WebView service's begin/progress/complete/cancel protocol and prove which
connection owns download policy. Treat another automation service as a separate
owned route if required; do not send guessed automation commands through
WebInspector. Retain original request/event IDs and check
browser-context/target/frame correlation.

When transferring files, admit an authorized host output root and
byte/concurrency/total/cleanup budgets; transfer actual bytes into owned
temporary files; publish final files only after confirmed completion.
Cancellation must stop the original source and preserve primary/cleanup failures.
A deny result must demonstrate the engine prevented a download. Allow/default
modes must prove their actual effects and reset behavior; unsupported modes retain
explicit errors.
