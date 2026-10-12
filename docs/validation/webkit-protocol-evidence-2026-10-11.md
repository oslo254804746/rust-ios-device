# F04 protocol evidence — 2026-10-11

This read-only investigation pins WebKit main to
`91e8fd1b2f0cec329c19c738cf90d470ac51be40`, obtained with `git ls-remote`.
The primary JSON definitions were fetched at that exact revision. Current
WebKit definitions do not certify availability on any particular iOS release.
F04 remains open; these observations constrain subsequent translations.

The inspected [Page protocol](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/JavaScriptCore/inspector/protocol/Page.json)
contains `getResourceTree`, `snapshotRect`, `snapshotNode` and `archive`, but
does not provide CDP navigation or download-control commands. Existing navigation
therefore depends on an in-page request and matching native commit event; a
Runtime return cannot stand in for that event.

[Automation](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/WebKit/UIProcess/Automation/Automation.json)
defines `performMouseInteraction`, `performKeyboardInteractions`,
`performInteractionSequence` and `cancelInteractionSequence` on automation
browsing-context handles. Keyboard interactions explicitly deliver native key
events. These are a separate automation session, not Inspector page messages.
Trusted CDP Input requires an owned automation attachment and a verified mapping
to the same selected page/context, with cancellation and cleanup. Forwarding an
Automation method over the Inspector socket, or synthesizing DOM events, cannot
close that requirement.

[Memory](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/JavaScriptCore/inspector/protocol/Memory.json)
provides enable/disable/startTracking/stopTracking;
[Heap](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/JavaScriptCore/inspector/protocol/Heap.json)
provides snapshot/tracking/GC/object inspection; and
[Timeline](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/JavaScriptCore/inspector/protocol/Timeline.json)
provides instrumentation events. None of these defines CDP `Memory.getDOMCounters`
or `Performance.getMetrics`. A future adapter needs real counter semantics,
timestamps, freshness and bounded collection rather than fabricated fields or
constant results. Unsupported exact counters remain errors.

[Network](https://github.com/WebKit/WebKit/blob/91e8fd1b2f0cec329c19c738cf90d470ac51be40/Source/JavaScriptCore/inspector/protocol/Network.json)
defines interception add/remove using the same URL/stage/caseSensitive/isRegex
tuple, plus enabled state and stage-specific continuation. The current adapter's
wildcard tuple must be tracked through partial enable, failure, disable and
disconnect; removal/rollback and paused-request cleanup remain unfinished.
No download begin/progress/complete/cancel protocol was established by these
definitions. Browser download support needs separate verified source evidence.

The original protocol JSON files and exact revision are retained under
`repair/webkit-protocol`; no reference implementation source was copied into
production. The web search endpoint was unavailable and GitHub's unauthenticated
REST endpoint was rate-limited; the exact Git ref and raw primary files succeeded.
