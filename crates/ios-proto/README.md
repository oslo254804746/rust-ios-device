# ios-proto

Protocol type definitions and codecs for iOS device communication.

This is a library crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- AFC, Lockdown, usbmuxd, DTX, XPC, OPACK, TLV, and TLS helper types.
- Zero-IO protocol primitives shared by the higher-level crates.
- NSKeyedArchiver encode/decode helpers for DTX and XCTest payloads.

## Install

```toml
[dependencies]
ios-proto = "0.1.1"
```

## Example

```rust
use ios_proto::lockdown::LockdownFrame;

let payload = plist::to_bytes_xml(&plist::Dictionary::new()).unwrap();
let framed = LockdownFrame::encode(&payload);
let header = framed[..4].try_into().unwrap();
assert_eq!(LockdownFrame::decode_length(header), payload.len() as u32);
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-proto>

## License

Licensed under either of Apache-2.0 or MIT at your option.
