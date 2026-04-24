# ios-cli

Command-line tool for iOS device management, tunneling, and service interaction.

This is a binary crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Lists devices, pairs with devices, starts tunnels, and runs service commands.
- Wraps the rust-ios-device workspace libraries in an end-user CLI named `ios`.
- Useful for diagnostics, development workflows, and automation around attached iOS devices.

## Install

```sh
cargo install ios-cli
```

## Example

```sh
cargo install ios-cli
ios --help
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-cli>

## License

Licensed under either of Apache-2.0 or MIT at your option.
