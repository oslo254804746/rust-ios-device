# Protocol notes

This document is a high-level map of protocol modules in this repository. It is not a wire-level specification.

Note: The `proto` module is `pub(crate)` — these paths describe internal organization, not public API. Public types are re-exported at the `ios_core` crate root.

## Implemented protocol modules

- `ios_core::proto::usbmuxd`: usbmuxd message types.
- `ios_core::proto::lockdown`: lockdown plist framing helpers.
- `ios_core::proto::afc`: Apple File Conduit packet structures.
- `ios_core::proto::dtx`: DTX message structures used by Instruments-style services.
- `ios_core::proto::xpc`: XPC and RemoteXPC values.
- `ios_core::proto::opack`: OPACK encoding/decoding.
- `ios_core::proto::nskeyedarchiver`: NSKeyedArchiver decoding.
- `ios_core::proto::nskeyedarchiver_encode`: NSKeyedArchiver encoding helpers.
- `ios_core::proto::tlv`: TLV utilities used by pairing flows.
- `ios_core::proto::tls`: shared TLS helper types.

## Service layers

- `ios_core::lockdown` builds on lockdown framing for sessions, service startup, pairing, and pair records.
- `ios_core::xpc` builds the RSD and RemoteXPC transport over HTTP/2.
- `ios_core::services` implements higher-level service clients such as AFC, syslog, screenshot, DTX/Instruments, TestManager, ImageMounter, WebInspector, and CoreDevice file/device information/diagnostics/configuration/orientation services.

CoreDevice configuration uses the action-only `CoreDevice.*` envelope from
`CoreDeviceService.invoke` (with `CoreDevice.actionIdentifier` and no feature
identifier). Orientation uses the raw `OrientationRequest` dictionary on
`com.apple.coredevice.devicecontrol`; its RSD feature is
`com.apple.coredevice.feature.remote.devicecontrol.orientation`.

Protocol compatibility should be treated as best effort. Apple can change private services and message shapes between iOS versions.

## Defensive decoding and transfer limits

Recent protocol handling rejects malformed DTX length, fragment, and identifier
metadata instead of accepting a truncated frame or panicking on counter
overflow. Pending fragmented messages and unrelated DTX replies are bounded so
a peer cannot grow host memory indefinitely. NSKeyedArchiver decoding accepts
the UID indirections and nullable XCTest fields seen in device responses, while
XCTest configuration encoding stores object-valued fields as keyed references.

InstallationProxy Browse stops with a protocol error after its bounded chunk or
entry budget. MobileBackup2 uses bounded file-transfer buffers and reports
available-space context for insufficient-space failures. These limits are
implementation safeguards; callers should surface the resulting protocol
errors and retry only after checking device/service state.
