# Actual pinned clients through the Rust CDP bridge

Accepted host boundary on Linux x86_64, 2026-10-09. Implementation: `3e697fd210f5276a59070ac7502e7991969a3ee4`.
This is the F03 client framework/default-context work item. Full Playwright and
physical page/JSContext/domain workflows retain their original acceptance.

## Delivered behavior and actual outcomes

BrowserCdpState consistently identifies its single default logical context as
`<browser_id>:default` in target listing, discovery, getTargetInfo and attachment.
The identity survives refresh. It describes this bridge's target namespace;
it does not promise WebKit storage isolation or context creation. Existing
getBrowserContexts returns only created contexts (empty), and create/dispose
remain explicit -32601. The additive TargetInfo field follows the
[CDP Target schema](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Target.pdl).

Node v24.19.0/npm11.17.0 ran pinned puppeteer-core25.12.0 and playwright-core1.63.0
against the production authenticated/protected HTTP/WS router. Native peers are
owned synthetic WebInspector plist JSContexts, not a standalone CDP server.
No client method or response is overridden; Playwright's real pw:protocol diagnostic
sink and Puppeteer's public logger capture actual bounded messages.

Puppeteer completed initialization, URL target discovery, flat-session attachment,
Runtime.enable, actual executionContextCreated selection, 6*7 evaluation=42,
unknown context rejection (-32602), detach and disconnect. The unknown context
is rejected before another native evaluation. Native session Target.setAutoAttach
has no JSContext Target domain and retains -32601; no empty success is invented.

Playwright's actual connectOverCDP reaches Browser.setDownloadBehavior and receives
the correlated original -32601. The negative contract gate passes; the client
initialization itself fails and its full workflow is not claimed as successful.
The capability observation records that incompatibility as FAIL. A separate
actual-child cancellation gate waits for consumed Runtime.enable before aborting
its waiter. The supervisor is not aborted: it kills and waits for the real Node
child, joins both bounded original pipes, then publishes cleanup evidence.

The framework records complete actual command/event/close order, max512 records
and256KiB per client/native logical trace,512KiB stdout/64KiB stderr. These logical
limits do not certify temporary parser allocations. Child total20s and cleanup5s
are explicit fixture policies. Original native peer tasks require actual zero-byte
EOF and join; fallback abort/timeouts fail acceptance. The production upgraded
WS source owner must release before peers are drained, because listener shutdown
alone does not join independently upgraded owners. Credentials/selectors are
checked absent from bounded machine evidence before behavior assertions.

## Failures retained and validation

Initial Rust compilation found three ambiguous Dictionary::from_iter `.into()`
keys; root supplied unambiguous &str keys without changing wire/assertions.
The first actual Playwright run exposed an earlier unhandled assertion on missing
browserContextId. This is why the product now identifies its existing default
context. It was not a download-error success: the failed log preserves the real
assertion and bounded stderr. The source remains JSContext for both clients;
no WebPage swap or client override hides that failure.

The initial full entry passed fmt and workspace all-target/all-feature check,
then strict Clippy rejected a redundant lifetime in a fixture helper. Root elided
it without changing assertions. Only that fixture differs between the two full
manifests; the initial full command exited1, not0. Root resumed fmt, strict lint
and every remaining required repository stage, serial/fail-fast: exit0 in402.201s.
The prior all-target check remains separate evidence. Primary Rust2954/Python13
passed, zero failed. Ordinary Rust ignores five tests: two existing and these
three explicit Node gates. Root then executed all three Node gates on the final
499-path frozen source:3PASS/0FAIL/0ignored,54.875s including compilation,3.40s test
runtime. They are not accepted merely because ordinary full skips them.

MSRV1.80.1,66 feature checks/four groups, default/full doctests7each, three
downstream examples, linked C ABI, rebuilt/installed wheel and sdist,73 tooling
tests,118 capability records,48 raw captures, five host workflows,95 CLI and
three backup contracts passed. Device rows remain22NOT_RUN. All live499 product
path hashes and committed blobs match the final full and explicit-client manifests.
Root alone ran Rust tools; independent agents did not compile.

Evidence: target/validation/cdp-client-workflow-2026-10-09/, including all failed
logs, both full manifests/results, resume-full.py, actual-client-reports.json,
frozen-implementation.patch (new files included) and accepted-evidence.json.

Resumed log SHA256: `6fab6d5eb8adbbc101ba2f2f239c4d9c2731e53d8b940d29b60a76d5144b40ce`.
Final actual-client log SHA256: `e8b4b591dab8aa9aca919fb2f2a3389d3f45344443f98729a450c7da581bdbea`.
Frozen patch SHA256: `edff74ca383c567b4b29c2e6baa16927d3964be7d248c0f866a004e77c410ea5`.
Actual client artifacts SHA256: `2f37ea258bd2163a5e24da54b92e9183350b3415d1dbb4897faec4199b3a2897`.
CLI SHA256: `ba1445ba71e5476d4b76443c8d37b0b991ec1ede42fc7b05472f6acfc0ee357d`.

[Reproduction and download implementation boundary](../cdp-client-workflow.md).
F03 remains in progress; F04 console/context destruction, page/iframe, input,
download/interception, metrics and representative physical workflows remain.
No main-task or maturity upgrade follows these aggregate tests. Selected authorized
hardware/iOS/USB/Wi-Fi and macOS/Windows conditions remain missing. Q04 and all
children remain excluded.
