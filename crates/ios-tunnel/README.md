# ios-tunnel

CDTunnel handshake and TUN forwarding utilities for iOS 17.4+ Remote Service Discovery.

This is a library crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Exchanges CDTunnel parameters and exposes Remote Service Discovery tunnel info.
- Supports userspace and kernel TUN management abstractions.
- Contains packet-forwarding helpers used by the CLI and high-level API.

## Install

```toml
[dependencies]
ios-tunnel = "0.1.1"
```

## Example

```rust,no_run
use ios_tunnel::{TunMode, TunnelManager};

let manager = TunnelManager::new(TunMode::Userspace);
# let _ = manager;
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-tunnel>

## License

Licensed under either of Apache-2.0 or MIT at your option.
