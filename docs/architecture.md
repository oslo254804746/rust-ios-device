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

## Backup boundaries

`services::backup2` keeps the existing public client, options, result types and
function reexports. Its private modules consume explicit streams, values or
owned layouts; moving a file does not publish new protocol behavior.

| Owner | Internal boundary |
| --- | --- |
| `protocol` | Exact request shapes, bounded framing, numeric decoding and diagnostic redaction; no filesystem or connection ownership. |
| `filter` | Bounded presets/regex predicates shared by receive-time and manifest selection. |
| `host_fs` | Root/path/symlink policy, metadata limits, staged files, disk operations and typed blocking workers; returns completed path changes for placeholder tracking. |
| `manifest` / `crypto` / `mbdb` | Feature-gated local index, keybag and legacy-index operations using the shared path policy. Public synchronous functions remain synchronous. |
| `session` | DeviceLink negotiation, transfer accounting, status order, placeholder tracking and best-effort disconnect; invokes host work off the async worker thread. |

Core and CLI await owned blocking jobs for normal metadata/path/commit/copy/
rename/remove/list work. Upload/download bytes continue through Tokio file I/O
and bounded buffers. Dropping a worker future does not stop a started blocking
job or undo its completed filesystem effects. A staged-file guard still drops
after the file handle and removes its own temporary on interrupted transfer;
its drop fallback may perform synchronous cleanup. L01 retains stronger deadline,
cancellation and physical-device cleanup acceptance. Existing transfer and
metadata budgets are unchanged. CLI local operations retain their result formats
and return output errors; see [backup output contracts](backup-contract.md).

## WebInspector boundaries

The `webinspector` public facade retains metadata, client/session types,
constants, errors and `cdp_browser` paths. Its private `wire` and `timing`
modules consume supplied streams/values and preserve the existing framing and
deadline budgets. `model` owns typed metadata parsing; `client` owns the stream,
application/page cache and event routing. `inspector` owns Target envelopes and
response state, and `automation` owns WebKit automation commands and element
semantics. These layers do not dial devices or serve HTTP.

The CLI facade delegates grammar/dispatch to `commands`, discovery/selection
and result interpretation to `workflows`, and one-shot/interactive execution to
`tasks` and `shell`. `http` owns listener startup and shared resources; `cdp`
owns debugger adaptation and child task guards; `webdriver` owns HTTP automation
sessions and element representations. Stored credential leases remain in
`sessions`, and safe protocol diagnostics remain separate from requested JS data.

`connection::InspectorSource` is an internal Rust interface returning an owned
service stream and optional device guard. Production startup always installs
`DeviceInspectorSource`, retaining the real guard across the complete session.
Owned synthetic tests may supply a stream without constructing a fake device.
HTTP/CLI inputs cannot select a synthetic source. The actual protected Rust
routers use this interface after admission, so F03 can exercise the bridge
with controlled plist peers. Host synthetic tests do not certify Apple devices
or external CDP client interoperability.

## Result conversion and output

`ios_core::plist_json` owns explicit historical Data/Date/Uid policies and
borrowed/owned iterative container conversion. Core services, CLI commands and
C lockdown rendering delegate ordinary conversion to it. MDM key-aware secret
redaction remains in its own traversal; direct plist serde and typed XPC/model
serialization retain their separate contracts. Conversion consumes constructed
values and does not add a transport, parser, redaction or hidden size limit.

CLI `output` owns fallible result lines, formatted prompts and XML stdout;
private display helpers propagate its IO errors to command dispatch. Backup
reexports the same line writer. Binary capture checks its direct writes and
final flush. Main reports existing failures through best-effort stderr while
retaining their failure status. See [encoding/output contracts and migration](
plist-json-contract.md); A02 classification/stream and L01 cleanup acceptance
remain distinct.

## Feature flags

`ios-core` keeps most service implementations behind feature flags. Examples include `afc`, `apps`, `syslog`, `screenshot`, `dtx`, `instruments`, `testmanager`, `debugserver`, `imagemounter`, `pcap`, `webinspector`, `fileservice`, and `deviceinfo`.

The CLI enables a broad set of service features because it exposes many commands. Library users should enable a narrower set where possible.

See [features.md](features.md) for the current grouping strategy.
