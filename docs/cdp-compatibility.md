# CDP compatibility status

The WebInspector bridge has browser Target discovery, flat sessions, and a
limited JavaScriptCore execution-context adapter. `Target.getBrowserContexts`
reports the single default context. `Target.setAutoAttach` applies the ordered
target-type filter sent by Puppeteer, including its page exclusion rule. A
synthetic WebInspector service test exercises JSContext `Runtime.enable`,
execution-context announcement, and `Runtime.evaluate` with the synthetic
context ID. These are protocol tests, not a passed Playwright or Puppeteer page
workflow.

R02 adds an internal Rust-only `InspectorSource` boundary shared by the actual
protected CDP and Selenium routers. Owned host tests now traverse the Rust page
and browser WebSockets through synthetic plist peers, covering discovery,
unpaused Target attachment, Runtime replies/errors, flattened session routing,
detach and teardown. See [module/injection acceptance](validation/webinspector-boundaries-2026-10-01.md).
The actual pinned-client framework now uses this production entry point.
Puppeteer25.12.0 completes a synthetic JSContext Runtime workflow; Playwright1.63.0
records actual Browser.setDownloadBehavior/-32601 initialization failure.
[Reproduction and scope](cdp-client-workflow.md). Physical Safari/WebView/JSContext
and the remaining domain workflows retain their acceptance.

The Rust CDP and Selenium servers now require `--auth-config` or explicit
loopback `--local-unauthenticated`. Native HTTP and WebSocket clients need the
scoped bearer header; `debug` must cover the server's selected device.
Expiry/reload revokes established sockets and pending requests. Host acceptance
checks only admission/metadata, including a WebSocket blocked before device
access; it does not supply client initialization or page workflow evidence.
Host/Origin checks are enforced; controlled browser clients use single-use CDP
handshake tickets. The S02 Chromium/manager byte-relay gate does not certify a
Playwright CDP session or an Apple page workflow. Debug
listeners require explicit UDID selection without first-device discovery. See
[HTTP boundaries](http-boundary.md) and
[management access](management-access.md) before migrating existing clients.

The following domains have limited adapters or still lack implementation:

| Domain | Missing evidence and next validation |
| --- | --- |
| JSContext Runtime | Actual Puppeteer selects the synthetic context through the Rust bridge, evaluates and rejects an unknown context ID. Parsed native page/application closure now reaches a typed session event and emits context destruction before detach/WS close; four added pinned-client closure gates require explicit execution. Playwright's narrow public noDefaults/nonflat CDPSession fixture leaves download policy alone; default initialization still rejects unsupported download control. Console workflows, full Playwright initialization and representative physical JSContext workflows remain pending. [Closure mapping, bounds and gate status](validation/cdp-native-context-closure-2026-10-13.md). |
| iframe / Page | The adapter uses native `Page.getResourceTree` and frame events, preserving children. Navigation waits for matching native frame/URL commit; it does not infer completion from evaluate returning. Actual pinned-client page/frame workflows remain pending. |
| Input / screenshot | PNG viewport/scale-1 clip screenshots use native `Page.snapshotRect` and validate decoded image data. Trusted input remains unsupported; DOM-synthesized events cannot establish native input semantics. Actual pinned-client screenshot workflows remain pending. |
| Fetch | One wildcard request/response stage uses native Network interception, with correlated pause/continue/fulfill/fail acknowledgements. Authentication/resource filters, complete rollback, target cancellation and downloads remain missing. Actual client interception workflows remain pending. |
| Memory / Performance | Capture supported WebInspector methods and result fields, then run client metric and memory requests against fixtures. Reject commands lacking a verified translation. |

The bridge returns CDP `-32601` for unsupported methods, including trusted Input,
Memory and Performance, rather than forwarding overlapping WebKit names.
Page/Fetch support is limited to the explicit translations above.
`Runtime.evaluate` with `throwOnSideEffect=true` also returns an
error because WebKit does not provide the required no-side-effect guarantee.
The JSContext adapter rejects unknown context IDs before stripping its own
synthetic context selector.

## Reproduce the client initialization probe

The repository's `scripts/cdp-client-probe.cjs` runs current client libraries
against a standalone fake CDP endpoint to record startup methods. It never
contacts a device or the Rust bridge. For example:

```sh
npm install --prefix /tmp/rust-ios-cdp-probe --no-audit --no-fund playwright-core@1.63.0 puppeteer-core@25.12.0 ws@8.22.0
NODE_PATH=/tmp/rust-ios-cdp-probe/node_modules node scripts/cdp-client-probe.cjs
```

With `playwright-core` 1.63.0 and `puppeteer-core` 25.12.0, Puppeteer first sent
`Target.getBrowserContexts`, `Target.setDiscoverTargets`, and
`Target.setAutoAttach` with a filter excluding page targets. It connected to
the fake endpoint but found zero pages. Playwright requested
`Browser.getVersion`, `Target.setAutoAttach`, `Browser.setDownloadBehavior`, and
`Target.getTargetInfo`; the probe rejected `Browser.setDownloadBehavior`, so
`connectOverCDP` failed. The bridge now accepts the trailing slash on
`/json/version/` used by Playwright's HTTP endpoint lookup. Download behavior
remains unimplemented; returning an empty success would claim unsupported
download handling. The [actual Rust bridge gate](cdp-client-workflow.md) now records pinned client
commands/events and original native/child close order. The standalone probe
remains request discovery only. Download translation and the remaining domain
workflows retain their explicit gaps.

The single default logical context now has a stable `<browser_id>:default`
TargetInfo.browserContextId across listing/discovery/attachment/refresh. This
fixes Playwright's earlier missing-context assertion, allowing it to reach the
explicit download rejection. It does not implement context creation or isolation.
