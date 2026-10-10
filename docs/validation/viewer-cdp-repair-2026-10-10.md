# Viewer/CDP review repair, 2026-10-10

This document records source changes after the af20f6a review. It does not reuse
the historical F04/F05 completion claims as acceptance evidence. Main-agent
serial Rust validation and the final tested commit must be recorded in the
combined repair report. No real device, restore, pairing or application install
was performed by this work package.

Viewer repair replaces XOR challenge response with RFC 6143 DES and an
independent PyCryptodome vector (`password`, challenge bytes 0..15 ->
`b866924125c8eebb9debc1db61c538e2`). Policy-mode RFB requires a scoped owner
token plus the legacy RFB password; HTTP checks exact UDID/listener-port scope.
Principal revision/expiry is retained through streams and device requests.
The production entry points start a shared bounded PNG decoder/producer and
an acknowledged HID/text pasteboard worker, own all task joins, and preserve
primary/cleanup errors. Display's original drain/RTCP tasks are aborted and
joined by `MediaStreamSession::stop` before it returns.

New regressions cover DES, wrong device/port, absent/failing backend, real text
clipboard responses, blocked handshake shutdown, actual PNG pixels/bounds,
and media task retirement. These tests have been added; their execution is a
main-agent responsibility and is not claimed by this document.

CDP no longer intercepts requests with fixed frame IDs, example URLs, 1x1
screenshots, timestamps, metrics or unconditional Fetch/input successes.
`PageCdpAdapter` owns bounded pending IDs and uses real device responses:

- Page enable/disable receive native acknowledgements; frame-tree uses
  `Page.getResourceTree` and retains native same/cross-origin iframe IDs.
- Navigation reads the real frame tree, issues in-page location assignment and
  replies only after a matching real frame commit or same-document event.
  It cannot infer successful navigation from evaluation alone. Redirect/target
  swap/native-client acceptance remains pending.
- PNG screenshots obtain the device viewport (or a scale-1 integer clip), call
  `Page.snapshotRect`, validate/decode the actual PNG data URL and return its
  original base64. JPEG, quality/unsupported capture options are refused.
- Fetch uses `Network.addInterception`, `setInterceptionEnabled`, native
  interception events and `intercept*` acknowledgements. Unknown request IDs,
  overlapping resumes and unsupported overrides/auth/filter semantics are
  refused. The supported subset is one wildcard stage pattern; complete
  enable/disable rollback, target-swap/cancel/download workflows remain gaps.
- Standard Input, Memory and Performance are refused with -32601. DOM script
  synthesis cannot meet trusted CDP input semantics and is not exposed as a
  successful standard Input implementation. No metric constants are emitted.
- JSContext console timestamps are converted from observed native seconds to
  CDP milliseconds. Context destruction/clear methods are used for native
  Inspector/Target and Runtime clear events, instead of only test calls.

Both direct page and flat browser page bridges use the same adapter, poll its
absolute deadlines and forward wire follow-ups. Protocol basis is the pinned
read-only pymobiledevice3 `cdp_target.py` at
`03ba2270c0c80df77c37f65adc1967595970d31f`, plus existing local Inspector
transport ownership. Added tests check native IDs, acknowledgement order,
navigation commit, actual PNG response, unknown/failed/retried Fetch request
IDs, explicit unsupported domains and late-response timeout handling.

F04 is still incomplete: actual pinned-client domain workflows, native trusted
Input, complete Fetch/download/cancellation, Memory/Performance fields/units,
and representative physical page/JSContext behavior remain outstanding.
F05 gaps are listed in [viewer usage](../viewer.md). Host unit/protocol results,
actual standard VNC clients, browser behavior and physical/platform acceptance
must remain separate. Physical/device/platform workflows are NOT_RUN.

Static maintenance executed by the repair agent: `scripts/capabilities.py`
passes with the directory CLI resolver and new experimental Viewer record;
`python -m unittest discover -s scripts -p test_capabilities.py` passes.
This is capability-inventory verification, not Viewer or CDP workflow evidence.
