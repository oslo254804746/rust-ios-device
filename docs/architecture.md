# Architecture

The workspace now exposes a small set of public crates. `ios-core` owns the
protocol, transport, service, discovery, pairing, and high-level Rust APIs.
`ios-cli`, `ios-py`, and `ios-ffi` are front ends built on top of it.

Note: Internal modules (`proto`, `mux`, `lockdown`, `xpc`, `tunnel`,
`pairing_transport`, `psk_tls`) are `pub(crate)`. Supported types are
re-exported at the `ios_core` crate root. The paths below describe internal
organization for contributors.

[Public compatibility rules](public-contract.md) govern source paths, struct/
enum evolution, command results and bindings. Internal file moves retain their
published reexports and are reviewed separately from behavior changes.

## Layers

`ios_core::proto` contains wire formats and codecs. It does not own device connections.

`ios_core::mux` talks to usbmuxd for device enumeration, event listening, and port forwarding to a device.

`ios_core::lockdown` implements lockdown request/response handling, pair records, TLS session setup, service startup, and pairing helpers.

`ios_core::tunnel` implements the CDTunnel handshake and packet forwarding for devices that expose the CoreDevice tunnel path. It supports userspace forwarding through smoltcp and kernel TUN devices through `tun-rs`.

`ios_core::xpc` implements Remote Service Discovery and the HTTP/2 + XPC transport used by CoreDevice-style services.

`ios_core::services` contains feature-gated service clients, including the
CoreDevice configuration and orientation clients. Higher-level commands
should enable only the features they need.

`ios-core` combines discovery, pairing material, lockdown service access, tunnel setup, and RSD/XPC service access into a higher-level API.

`ios-cli`, `ios-py`, and `ios-ffi` are user-facing front ends.

## Connection paths

Classic services generally follow:

```text
host -> usbmuxd -> device lockdown port -> StartService -> service port
```

CoreDevice tunnel/RSD services generally follow:

```text
host -> usbmuxd or remote pairing path -> CoreDeviceProxy/CDTunnel -> RSD -> RemoteXPC or raw service
```

Some services can be available through both paths depending on iOS version and device state.

`device` uses ordinary private submodules with explicit imports. The facade
retains its public types and connection reexports; the crate-root paths also
stay available. A source move does not change wire messages or advertised
device support.

| Owner | Internal boundary |
| --- | --- |
| `connection` | Public entry points select USB/TCP/direct-RSD/RemotePairing strategies using explicit options, identity and optional pair records. |
| `discovery_match` | Existing-trust Bonjour matching and bounded lockdown probes return paired device targets; address ordering is shared explicitly with RemotePairing discovery. |
| `lockdown_transport` | Pair-record loading, port dialing, lockdown session/service startup and the plain/TLS CoreDeviceProxy stream. |
| `tunnel_connect` | Direct-RSD/RemotePairing discovery and authenticated streams; nested credential, pairing and codec modules keep their narrow visibility. |
| `tunnel_activation` | Receives `TunnelConnection`, an authenticated async stream and `TunMode`; negotiates and activates the shared tunnel lifecycle. |
| `rsd_connect` / `tunnel_endpoint` | Optional RSD probing and direct/userspace socket routes; no lockdown credential or service-start ownership. |
| `service_routing` | Receives an RSD directory and requested name, returns the exact resolved name/port/features, or performs bounded RSD check-in on a supplied stream. |
| `connected` / `support` | Connected-device public service methods and typed lockdown value extraction. |

Tests live with their owning layer. Host fixtures cover selection, synthetic
control messages, userspace routing headers/RSD bootstrap, direct IPv6 endpoint
selection and bounded TCP lockdown byte exchange. Kernel TUN creation and Apple
service lifecycles still require their platform and physical-device evidence.

## Feature flags

`ios-core` keeps most service implementations behind feature flags. Examples include `afc`, `apps`, `syslog`, `screenshot`, `dtx`, `instruments`, `testmanager`, `debugserver`, `imagemounter`, `pcap`, `webinspector`, `fileservice`, and `deviceinfo`.

The CLI enables a broad set of service features because it exposes many commands. Library users should enable a narrower set where possible.

See [features.md](features.md) for the current grouping strategy.
