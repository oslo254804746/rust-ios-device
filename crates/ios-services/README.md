# ios-services

Feature-gated implementations of iOS device services.

This is a library crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Apple File Conduit, syslog, screenshots, app management, crash reports, and notifications.
- DTX-based Instruments, XCTest/TestManager, accessibility audit, and debug proxy support.
- CoreDevice/XPC services for newer iOS versions, image mounting, restore, and diagnostics.

## Install

```toml
[dependencies]
ios-services = "0.1.1"
```

## Example

```toml
[dependencies]
ios-services = { version = "0.1.1", features = ["afc", "syslog", "screenshot"] }
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-services>

## License

Licensed under either of Apache-2.0 or MIT at your option.
