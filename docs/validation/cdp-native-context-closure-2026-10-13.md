# F04 native JSContext closure slice — 2026-10-13

This change implements the native page/application closure part of F04-1. It does
not complete F04-1 or F04. The root agent owns compilation and explicit
client-gate execution, and completed the discovery retirement extension after
the implementing agent stopped on its usage limit.
No device discovery or physical device operation is part of this fixture.

## Evidence and mapping

At the existing WebKit pin `91e8fd1b2f0cec329c19c738cf90d470ac51be40`,
[Runtime](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/JavaScriptCore/inspector/protocol/Runtime.json)
defines executionContextCreated but no executionContextDestroyed/ContextsCleared;
[Inspector](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/JavaScriptCore/inspector/protocol/Inspector.json)
defines no detached event. Both primary definitions were re-opened at that exact
revision on 2026-10-13. These schemas do not prove any particular iOS release.

The bridge consumes the existing parsed native plist Listing and
ApplicationDisconnected events. A Listing is admitted only after its complete
WIRListingKey dictionary and every Page parse succeed. Exact application identity
plus absence of the selected numeric WIRPageIdentifierKey means closure; changing
a listing key while keeping the selected numeric page does not. A missing,
malformed, or partial listing is an error, not positive closure evidence. An
application disconnect closes only sessions for that exact application.

InspectorSession::next_session_event returns typed Message/Closed outcomes.
next_raw_message preserves its JSON return type and reports native closure as a
protocol error. IO, partial-frame, EOF and parser errors retain their original
sources. Unrelated native/socket events remain ordered under the existing
1024-event/16 MiB compact-JSON policy. Empty idle expiry restores completed
unrelated events and permits reuse; interruption after accepted bytes retains
the client invalidation rule.

The JSContext adapter emits one CDP executionContextDestroyed only if Runtime
enable previously announced the context. Its numeric and unique identifiers
match the announced context. The
[CDP Runtime schema](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/json/js_protocol.json)
defines both destruction identifiers. Unsupported/unknown contexts continue to
return -32602 before native evaluation. Closure follows native plist evidence;
guessed Inspector.detached or Runtime.executionContextsCleared events provide
no substitute.

For a direct page WebSocket, destruction is flushed before the Close frame.
For browser sessions, the outcome retains one small destruction candidate only
after the created event enters the admitted queue. Terminal Notify avoids a
join waiting on a full message channel. The browser joins the original child,
drains its queued messages in order, flushes destruction and emits
Target.detachedFromTarget while its exact UUID route still exists. Independent
discovery follows the same retirement helper; listing-key changes retain the
same (application, numeric page) identity.
Native teardown, original peer EOF, task joins and primary/cleanup causes remain
separate evidence. Channel and deferred queue share the original 64 admission
permits. Deferred entries retain their permit; final lifecycle data is one
bounded candidate per admitted session. Join, queued-message flush, destruction
and detach share one absolute two-second retirement budget. Native failures
remain original cleanup outcomes; fallback abort is followed by an original
task join and does not certify device cleanup.

## Actual clients and host transport

The added gates run the existing production protected Rust router and its owned
synthetic WebInspector source with pinned Puppeteer 25.12.0 and Playwright 1.63.0.
Neither client is replaced by a standalone CDP server or overridden methods.

Puppeteer uses its public flat CDPSession. Playwright uses public
connectOverCDP({noDefaults:true}), newBrowserCDPSession and the standard nonflat
Target attachment/send/received-message transport. noDefaults is explicitly
declared by the installed pinned client. This narrow choice leaves download
policy at the browser default; it does not establish default Playwright
initialization compatibility. The original default connectOverCDP rejection at
Browser.setDownloadBehavior/-32601 remains an independent unchanged gate.

The host nonflat envelope follows the
[official CDP Target protocol](https://raw.githubusercontent.com/ChromeDevTools/devtools-protocol/master/pdl/domains/Target.pdl):
a selected session receives JSON messages through sendMessageToTarget and returns
them through receivedMessageFromTarget. The browser-session owner and target are
correlated; unknown/wrong-parent sessions return -32001, malformed nested JSON
returns -32602, and unsupported semantic methods retain their nested error.
This adds CDP transport routing, not WebKit commands. Serialized outer messages
are limited to 1 MiB, including the nested string; channel capacity remains 64
and admission/final-native-relay waits have a two-second bound. These logical
limits do not measure parser peak allocation or RSS.

The fixture first enables Runtime, selects the announced context, evaluates 6*7,
and rejects an unknown context. It then proves unrelated application/listing/data
signals preserve the session using a correlated native evaluation reply before
triggering exact selected-page or application closure. Each client receives one
destruction event with matching IDs before its selected session detaches. Original
plist send/teardown/zero-byte EOF/task-join and actual child wait/pipe joins remain
recorded by the existing bounded evidence owners.

## Required serial gates and status

Root serial outcomes: core WebInspector110 PASS; CLI WebInspector184 PASS /
10 explicitly ignored client gates in the ordinary run; the explicit pinned
client gate10 PASS (three original F03, seven closure). Strict all-feature,
all-target Core/CLI Clippy, CLI build, fmt/diff pass. Actual read-only manager1
passes with zero Source connections; host runner5 PASS / 22 NOT_RUN / 0 FAIL,
real_device_executed=false. Cached pinned Node dependencies were used without
installing dependencies or downloading browsers.

```powershell
cargo fmt --all -- --check
cargo test -p ios-core --all-features --locked native_inspector_ -- --test-threads=1
cargo test -p ios-core --all-features --locked nonflat_sessions_ -- --test-threads=1
cargo test -p ios-core --all-features --locked js_context_native_closure_ -- --test-threads=1
cargo test -p ios-cli --all-features --locked cmd::webinspector::cdp:: -- --test-threads=1
cargo test -p ios-cli --all-features --locked cmd::webinspector::bridge_tests::client_tests:: -- --ignored --nocapture --test-threads=1
```

The last command explicitly executes the three original F03 gates and seven
closure gates, including independent discovery removal and selected-stream
competition. Ordinary cargo test ignores dependency-requiring client
tests and cannot establish their acceptance. Full required repository checks
remain the root's responsibility.

The first root extension compile failed on a missing reference and a retained
test helper; both were corrected. The first actual-client run passed the three
original gates and failed all seven closure gates at the unchanged native
setup/teardown assertion. A completed unrelated event was held by a committed
outer read guard, so the next inbound client message invalidated the stream
despite no partial frame. The corrected guard checkpoints completed retained
event boundaries and restores the same bounded events on cancellation. Partial
native frame/read/routing cancellation retains the original invalidation rule.
The new boundary-cancel test and all 110 core WebInspector tests pass. Failed
logs and original byte traces are retained externally.

Strict Core Clippy initially rejected one collapsible branch and two test
construction styles. Corrections preserve behavior; the successful 110/184/10
gates are reused by impact after those style-only changes. The final build-source
manifest contains552 files, including independently committed L02 retry code.
Raw Git blob/source/binary proof for the final combined HEAD is recorded in
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\f04-native-closure\final-build`;
successful logs live in `verified`, prior failed logs in `extended` and `final`.
Other platforms, physical devices, feature/MSRV/SDK matrices were not rerun.

Remaining F04 gaps include Runtime console/client semantics and representative
physical JSContext/Safari/WebView workflows, full/default Playwright
initialization and download semantics, Page/iframe, trusted Input/screenshots,
complete Fetch cancellation/rollback/download and Memory/Performance fields and
lifecycle mapping. This closure slice changes no aggregate capability maturity.
