# CDP compatibility status

The protected Rust HTTP/WebSocket bridge routes browser Target discovery, flat
and bounded nonflat sessions to an injectable WebInspector source. Pinned
Puppeteer 25.12.0, Playwright 1.63.0 (including default `connectOverCDP`), and
Selenium 4.35.0 workflows connect through this production bridge to synthetic
plist peers and physical Safari/WebInspector targets on iOS 17–27.2. The single
default context has stable `<browser_id>:default` identity; creating isolated
browser contexts is unsupported.

Debug servers require an explicit UDID and `--auth-config` or explicit loopback
`--local-unauthenticated`. Bearer scope must cover `debug` for that device.
Expiry/reload revokes sockets and pending requests. Host/Origin checks and
single-use browser handshake tickets remain enforced. See
[HTTP boundaries](http-boundary.md), [management access](management-access.md)
and [client reproduction](cdp-client-workflow.md).

| Domain | Implemented mapping and remaining scope |
| --- | --- |
| JSContext / Runtime | Single implicit JSC realm supports evaluation, unknown-ID rejection, observed destruction before detach and console with device WallTime converted to milliseconds. Page normal/user/internal worlds keep native id/name/frameId; CDP origin comes from native frame.securityOrigin and auxData distinguishes default and isolated frame worlds. Missing origin triggers a real getResourceTree query; failure retires the inspector owner. Retired queries and stale tree snapshots cannot revive old realms or overwrite new origins. Navigation/clear/detach destroys or invalidates old contexts. Unknown future world types are filtered. Object console parameters prove a live world through their native injectedScriptId, and primitive-only console messages (`console.log/info/warn/error/debug`) synthesize `Runtime.consoleAPICalled` against the active main-frame default context when observed. |
| Page / iframe | Native frame-tree IDs and normal-world mappings support same/cross-origin child evaluation and navigation, with actual client gates. Navigation success waits for matching native frame/URL commit; an evaluate reply is insufficient. Referrer/transition options are rejected. Queries spanning tree changes retry under their original deadline. |
| Input / screenshot | PNG viewport and scale-1 clip screenshots use Page.snapshotRect and validate decoded PNG. Controlled `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`, `Input.insertText`, and `Input.dispatchTouchEvent` are supported. JPEG screenshots remain unsupported. |
| Fetch / downloads | Native wildcard request/response interception supports correlated pause, continue with URL/method/headers/base64 body, fulfill, fail and pending-request disable. Filters are removed for their exact stage/regex; failed/expired rollback retains primary plus cleanup error. Authentication/resource-type filters remain unsupported. Controlled `Page.setDownloadBehavior` and `Browser.setDownloadBehavior` compatibility is supported for client initialization and policy state. |
| Memory / Performance | Performance enable/start and stop/disable use native Memory sampling lifecycle. Timestamp is inspector elapsed seconds; WebKitJavaScriptMemory/JIT/Image/Layer/Page/OtherMemory are native allocation bytes. Missing categories are omitted, never zero-filled. JavaScript includes GCHeap plus GCOwned and is not Chrome JSHeapUsedSize/TotalSize. HeapProfiler.collectGarbage uses Heap.gc. DOM/listener counters and other unknown metrics are explicitly unsupported. |

Unsupported methods return -32601, unsupported parameters an explicit error.
`Runtime.evaluate(throwOnSideEffect=true)` is refused because WebKit has no
matching guarantee. Overlapping method names are not blindly forwarded.

The [capability inventory](capabilities.md) records the fixed-client gates,
native schema, lifecycle evidence, and physical-device Safari/WebInspector
acceptance through iOS 27.2.

`scripts/cdp-client-probe.cjs` is only an independent startup-method discovery
probe. It never contacts the Rust bridge or a device and is not acceptance of
any domain. Historical reports describe earlier implemented/rejected subsets.
