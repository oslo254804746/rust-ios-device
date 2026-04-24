# Usage

## CLI conventions

The binary is named `ios`.

Global options:

```sh
ios --help
ios -u <UDID> <command>
IOS_UDID=<UDID> ios <command>
ios --no-json <command>
ios -v <command>
```

Most commands default to JSON output unless `--no-json` is passed.

## Common commands

```sh
ios list
ios listen
ios discover mobdev2
ios -u <UDID> info
ios -u <UDID> lockdown get --key ProductVersion
ios -u <UDID> batterycheck
ios -u <UDID> syslog
ios -u <UDID> screenshot --output screenshot.png
ios -u <UDID> file ls /
ios -u <UDID> apps list
ios -u <UDID> crash list
ios -u <UDID> diagnostics info
ios -u <UDID> instruments cpu
```

Use command help for exact arguments:

```sh
ios file --help
ios apps --help
ios backup --help
ios instruments --help
```

## Rust API

Use `ios-core` for a high-level entry point:

```rust
use ios_core::{ConnectOptions, list_devices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = list_devices().await?;
    let device = devices.first().ok_or("no device found")?;
    let connected = ios_core::connect(&device.udid, ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    }).await?;

    println!("{:?}", connected.lockdown_get_value(Some("DeviceName")).await?);
    Ok(())
}
```

Use lower-level crates when you need direct control over usbmux, lockdown sessions, service startup, DTX, or XPC.

## Safety notes

Commands that install profiles, modify proxy settings, simulate location, erase, restore, or manage supervised state can alter device behavior. Prefer a test device and inspect `--help` before running them.
