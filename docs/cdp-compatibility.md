# CDP compatibility status

The protected Rust HTTP/WebSocket bridge routes browser Target discovery, flat
and bounded nonflat sessions to an injectable WebInspector source. Actual fixed
Puppeteer 25.12.0 and Playwright 1.63.0 tests connect through this production
bridge to synthetic plist peers. The single default context has stable
`<browser_id>:default` identity; creating isolated browser contexts is unsupported.
Default Playwright `connectOverCDP` still fails explicitly on
`Browser.setDownloadBehavior` (-32601). Its narrow noDefaults/nonflat session
workflow does not establish default browser compatibility.

Debug servers require an explicit UDID and `--auth-config` or explicit loopback
`--local-unauthenticated`. Bearer scope must cover `debug` for that device.
Expiry/reload revokes sockets and pending requests. Host/Origin checks and
single-use browser handshake tickets remain enforced. See
[HTTP boundaries](http-boundary.md), [management access](management-access.md)
and [client reproduction](cdp-client-workflow.md).

| Domain | Implemented mapping and remaining scope |
| --- | --- |
| JSContext / Runtime | Single implicit JSC realm supports evaluation, unknown-ID rejection, observed destruction before detach and console with device WallTime converted to milliseconds. Page normal/user/internal worlds keep native id/name/frameId; CDP origin comes from native frame.securityOrigin and auxData distinguishes default and isolated frame worlds. Missing origin triggers a real getResourceTree query; failure retires the inspector owner. Retired queries and stale tree snapshots cannot revive old realms or overwrite new origins. Navigation/clear/detach destroys or invalidates old contexts. Unknown future world types are filtered. Object console parameters can prove a live world through their native injectedScriptId, with finite native WallTime; only that subset adds Runtime.consoleAPICalled. Primitive-only, conflicting, retired or reused-ID console events remain native Console.messageAdded without guessed context or host time. |
| Page / iframe | Native frame-tree IDs and normal-world mappings support same/cross-origin child evaluation and navigation, with actual client gates. Navigation success waits for matching native frame/URL commit; an evaluate reply is insufficient. Referrer/transition options are rejected. Queries spanning tree changes retry under their original deadline. |
| Input / screenshot | PNG viewport and scale-1 clip screenshots use Page.snapshotRect and validate decoded PNG. JPEG screenshots and trusted Input remain unsupported. Selected-page focus, CSS-to-screen coordinates, direction and native click/key/modifier guarantees are not established by global HID or DOM dispatchEvent. |
| Fetch / downloads | Native wildcard request/response interception supports correlated pause, continue with URL/method/headers/base64 body, fulfill, fail and pending-request disable. Filters are removed for their exact stage/regex; failed/expired rollback retains primary plus cleanup error. Authentication/resource-type filters remain unsupported. Download control/save/cancel/events and default Browser.setDownloadBehavior remain OPEN; pinned WebKit Browser.json provides no verified equivalent. |
| Memory / Performance | Performance enable/start and stop/disable use native Memory sampling lifecycle. Timestamp is inspector elapsed seconds; WebKitJavaScriptMemory/JIT/Image/Layer/Page/OtherMemory are native allocation bytes. Missing categories are omitted, never zero-filled. JavaScript includes GCHeap plus GCOwned and is not Chrome JSHeapUsedSize/TotalSize. HeapProfiler.collectGarbage uses Heap.gc. DOM/listener counters and other unknown metrics are explicitly unsupported. |

Unsupported methods return -32601, unsupported parameters an explicit error.
`Runtime.evaluate(throwOnSideEffect=true)` is refused because WebKit has no
matching guarantee. Overlapping method names are not blindly forwarded.

The [current report](validation/production-completion-2026-10-26.md) records the
nonzero fixed-client gates, native schema and lifecycle evidence. Their fake
services validate known WebKit fields, not previously unknown protocol claims.
Physical Safari/WebView/JSC and other-platform execution remain NOT_RUN. F04
stays open for console messages without native world evidence, trusted Input and downloads.

`scripts/cdp-client-probe.cjs` is only an independent startup-method discovery
probe. It never contacts the Rust bridge or a device and is not acceptance of
any domain. Historical reports describe earlier implemented/rejected subsets.
