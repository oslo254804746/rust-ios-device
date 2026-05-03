# ios-cli

Command-line tool for iOS device management, tunneling, and service interaction.

This is a binary crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Lists devices, pairs with devices, starts tunnels, and runs service commands.
- Wraps the rust-ios-device workspace libraries in an end-user CLI named `ios`.
- Useful for diagnostics, development workflows, and automation around attached iOS devices.
- Covers common task families from go-ios and pymobiledevice3, including AFC
  file access, app management, syslog, pcap, crash reports, Developer Disk Image
  mounting, Instruments/DTX, WebInspector, CoreDevice tunnels, RSD, profiles,
  provisioning profiles, restore helpers, and supervision workflows.

## Install

```sh
cargo install ios-cli
```

## Example

```sh
cargo install ios-cli
ios --help
ios list
ios -u <UDID> info
ios -u <UDID> tunnel start --userspace
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-cli>
- Usage guide: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/usage.md>
- CLI map: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/cli-map.md>

## License

Licensed under either of Apache-2.0 or MIT at your option.
