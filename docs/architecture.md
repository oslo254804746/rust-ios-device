# Architecture

The workspace now exposes a small set of public crates. `ios-core` owns the
protocol, transport, service, discovery, pairing, and high-level Rust APIs.
`ios-cli`, `ios-py`, and `ios-ffi` are front ends built on top of it.

## Layers

`ios_core::proto` contains wire formats and codecs. It does not own device connections.

`ios_core::mux` talks to usbmuxd for device enumeration, event listening, and port forwarding to a device.

`ios_core::lockdown` implements lockdown request/response handling, pair records, TLS session setup, service startup, and pairing helpers.

`ios_core::tunnel` implements the iOS 17+ CDTunnel handshake and packet forwarding. It supports userspace forwarding through smoltcp and kernel TUN devices through `tun-rs`.

`ios_core::xpc` implements Remote Service Discovery and the HTTP/2 + XPC transport used by CoreDevice-style services.

`ios_core::services` contains feature-gated service clients. Higher-level commands should enable only the features they need.

`ios-core` combines discovery, pairing material, lockdown service access, tunnel setup, and RSD/XPC service access into a higher-level API.

`ios-cli`, `ios-py`, and `ios-ffi` are user-facing front ends.

## Connection paths

Classic services generally follow:

```text
host -> usbmuxd -> device lockdown port -> StartService -> service port
```

iOS 17+ tunnel/RSD services generally follow:

```text
host -> usbmuxd or remote pairing path -> CoreDeviceProxy/CDTunnel -> RSD -> RemoteXPC or raw service
```

Some services can be available through both paths depending on iOS version and device state.

## Feature flags

`ios-core` keeps most service implementations behind feature flags. Examples include `afc`, `apps`, `syslog`, `screenshot`, `dtx`, `instruments`, `testmanager`, `debugserver`, `imagemounter`, `pcap`, `webinspector`, `fileservice`, and `deviceinfo`.

The CLI enables a broad set of service features because it exposes many commands. Library users should enable a narrower set where possible.
