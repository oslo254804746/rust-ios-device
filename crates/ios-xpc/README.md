# ios-xpc

Raw HTTP/2 + XPC transport and Remote Service Discovery handshake support.

This is a library crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- RemoteXPC and XPC message transport building blocks.
- HTTP/2 framing for iOS Remote Service Discovery services.
- Used by CoreDevice-era services and higher-level service crates.

## Install

```toml
[dependencies]
ios-xpc = "0.1.1"
```

## Example

```rust
use ios_proto::xpc::{XpcMessage, XpcValue};

let message = XpcMessage {
    flags: 0,
    body: XpcValue::Dictionary(Default::default()),
};
# let _ = message;
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-xpc>

## License

Licensed under either of Apache-2.0 or MIT at your option.
