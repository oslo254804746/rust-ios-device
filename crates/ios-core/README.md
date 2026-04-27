# ios-core

High-level Rust API for discovering, pairing, connecting to, and controlling iOS devices.

This is a library crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Device discovery and connection orchestration across usbmuxd, lockdown, tunnel, and XPC layers.
- Pairing, tunnel startup, and service helpers suitable for applications and tools.
- Convenience API used by the CLI, FFI, and Python bindings.

## Install

```toml
[dependencies]
ios-core = "0.1.1"
```

## Example

```rust,no_run
use ios_core::{ConnectOptions, list_devices};

# async fn run() -> anyhow::Result<()> {
let devices = list_devices().await?;
println!("found {} device(s)", devices.len());

if let Some(device) = devices.first() {
    let connected = ios_core::connect(&device.udid, ConnectOptions::default()).await?;
    println!("connected to {}", connected.info.udid);
}
# Ok(())
# }
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-core>

## License

Licensed under either of Apache-2.0 or MIT at your option.
