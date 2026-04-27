# Protocol notes

This document is a high-level map of protocol modules in this repository. It is not a wire-level specification.

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
- `ios_core::services` implements higher-level service clients such as AFC, syslog, screenshot, DTX/Instruments, TestManager, ImageMounter, WebInspector, and CoreDevice file/device information services.

Protocol compatibility should be treated as best effort. Apple can change private services and message shapes between iOS versions.
