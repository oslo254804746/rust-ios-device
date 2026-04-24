# ios-ffi

C FFI bindings for the rust-ios-device high-level API and tunnel support.

This is a FFI crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Builds `cdylib` and `staticlib` artifacts for C-compatible consumers.
- Exposes device listing, pairing, service access, and tunnel lifecycle functions.
- Published as release artifacts rather than a crates.io package.

## Install

This crate is not published directly to crates.io. Build it from the workspace or use the release artifacts documented in the repository.

## Example

```sh
cargo build --release -p ios-ffi
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>

## License

Licensed under either of Apache-2.0 or MIT at your option.
