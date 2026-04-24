# ios-mux

Async usbmuxd client for iOS device discovery and connection multiplexing.

This is a library crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Connects to the local usbmuxd daemon on supported platforms.
- Lists attached iOS devices and opens per-device TCP-style channels.
- Provides the transport used by lockdown, services, tunnel, and core APIs.

## Install

```toml
[dependencies]
ios-mux = "0.1.1"
```

## Example

```rust,no_run
use ios_mux::UsbmuxClient;

# async fn run() -> anyhow::Result<()> {
let mut client = UsbmuxClient::connect().await?;
let devices = client.list_devices().await?;
for device in devices {
    println!("{}", device.udid);
}
# Ok(())
# }
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-mux>

## License

Licensed under either of Apache-2.0 or MIT at your option.
