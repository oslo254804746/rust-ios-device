# Protocol notes

This document is a high-level map of protocol modules in this repository. It is not a wire-level specification.

## Implemented protocol modules

- `ios-proto::usbmuxd`: usbmuxd message types.
- `ios-proto::lockdown`: lockdown plist framing helpers.
- `ios-proto::afc`: Apple File Conduit packet structures.
- `ios-proto::dtx`: DTX message structures used by Instruments-style services.
- `ios-proto::xpc`: XPC and RemoteXPC values.
- `ios-proto::opack`: OPACK encoding/decoding.
- `ios-proto::nskeyedarchiver`: NSKeyedArchiver decoding.
- `ios-proto::nskeyedarchiver_encode`: NSKeyedArchiver encoding helpers.
- `ios-proto::tlv`: TLV utilities used by pairing flows.
- `ios-proto::tls`: shared TLS helper types.

## Service layers

- `ios-lockdown` builds on lockdown framing for sessions, service startup, pairing, and pair records.
- `ios-xpc` builds the RSD and RemoteXPC transport over HTTP/2.
- `ios-services` implements higher-level service clients such as AFC, syslog, screenshot, DTX/Instruments, TestManager, ImageMounter, WebInspector, and CoreDevice file/device information services.

Protocol compatibility should be treated as best effort. Apple can change private services and message shapes between iOS versions.
