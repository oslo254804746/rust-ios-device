# CDP compatibility status

The WebInspector bridge has browser Target discovery, flat sessions, and a
limited JavaScriptCore execution-context adapter. `Target.getBrowserContexts`
reports the single default context. `Target.setAutoAttach` applies the ordered
target-type filter sent by Puppeteer, including its page exclusion rule. A
synthetic WebInspector service test exercises JSContext `Runtime.enable`,
execution-context announcement, and `Runtime.evaluate` with the synthetic
context ID. These are protocol tests, not a passed Playwright or Puppeteer page
workflow.

The Rust CDP and Selenium servers now require `--auth-config` or explicit
loopback `--local-unauthenticated`. Native HTTP and WebSocket clients need the
scoped bearer header; `debug` must cover the server's selected device.
Expiry/reload revokes established sockets and pending requests. Host acceptance
checks only admission/metadata, including a WebSocket blocked before device
access; it does not supply client initialization or page workflow evidence.
Browser handshakes and Host/Origin controls remain S02. See
[management access](management-access.md) before migrating existing clients.

The following CDP domains remain **unimplemented** as compatible domains:

| Domain | Missing evidence and next validation |
| --- | --- |
| JSContext Runtime | The synthetic WebInspector sequence passes, but no real Playwright/Puppeteer session has selected a JavaScript target through the Rust bridge. Capture that client sequence and check context selection, evaluation, and console events before declaring compatibility. |
| iframe / Page | Capture WebInspector frame-tree, navigation, and execution-context events from a page with a same-origin and cross-origin iframe. Run an actual client frame selection and navigation sequence through an injectable fake WebInspector service, then compare IDs and event order. |
| Input / screenshot | Capture WebInspector touch/key and screenshot requests plus image metadata. Run a client click, type, and screenshot sequence against a synthetic page, checking coordinates, modifiers, image encoding, and failures. |
| Fetch | Capture interception enable, paused request, continue/fulfill/fail, and cancellation events. Test request interception and response delivery with a real client against a fake service. |
| Memory / Performance | Capture supported WebInspector methods and result fields, then run client metric and memory requests against fixtures. Reject commands lacking a verified translation. |

The bridge returns CDP `-32601` for Page, Input, Fetch, Memory, and Performance
methods instead of forwarding overlapping WebKit names and possibly returning a
false success. `Runtime.evaluate` with `throwOnSideEffect=true` also returns an
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
download handling. The next end-to-end gate is to run these clients against
the Rust bridge with an injectable synthetic WebInspector service and record
the complete command/event flow for each domain above.
