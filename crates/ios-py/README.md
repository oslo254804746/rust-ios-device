# ios-py

Python bindings for rust-ios-device built with PyO3 and maturin.

This is a Python extension crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Provides Python access to selected high-level iOS device APIs.
- Uses the `ios_rs` native module name.
- Built and published through the Python wheel workflow instead of crates.io.

## Install

This crate is not published directly to crates.io. Build it from the workspace or use the release artifacts documented in the repository.

## Example

```sh
cd crates/ios-py
uvx maturin develop
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>

## License

Licensed under either of Apache-2.0 or MIT at your option.
