# Usage

The binary is named `ios`. Most commands print JSON by default so they can be
used from scripts. Pass `--no-json` for a more human-readable table or text
format where a command supports it.

## CLI conventions

```sh
ios --help
ios -u <UDID> <command>
IOS_UDID=<UDID> ios <command>
ios --no-json <command>
ios -v <command>
```

Use command help for exact arguments:

```sh
ios file --help
ios apps --help
ios backup --help
ios tunnel --help
ios instruments --help
```

## Device discovery and pairing

```sh
ios list
ios listen
ios discover mobdev2
ios -u <UDID> pair
ios -u <UDID> pair show-record
ios -u <UDID> lockdown info
ios -u <UDID> lockdown get --key ProductVersion
ios -u <UDID> lockdown save-pair-record pair-record.plist
```

Comparable upstream workflows:

- go-ios: `ios list`, `ios listen`, `ios pair`, `ios lockdown get`.
- pymobiledevice3: `pymobiledevice3 usbmux list`,
  `pymobiledevice3 lockdown ...`, `pymobiledevice3 bonjour rsd`.

Pair records are credentials for device access. Do not commit them or include
them in logs.

## Device information and lockdown values

```sh
ios -u <UDID> info
ios -u <UDID> diskspace
ios -u <UDID> mobilegestalt ProductType ProductVersion
ios -u <UDID> batterycheck
ios -u <UDID> batteryregistry
ios -u <UDID> activation state
```

Use `lockdown get` when you know the lockdown domain or key, and use the
higher-level commands when you want a narrower, typed view of common data.

## Files, app containers, and crash reports

The `file` command uses AFC by default:

```sh
ios -u <UDID> file ls /
ios -u <UDID> file tree /
ios -u <UDID> file pull /DCIM ./dcim
ios -u <UDID> file push local.txt /Downloads/local.txt
ios -u <UDID> file stat /Downloads/local.txt
ios -u <UDID> file rm /Downloads/local.txt
```

Use House Arrest for an app container:

```sh
ios -u <UDID> file --app com.example.app ls /
ios -u <UDID> file --app com.example.app --documents pull / ./Documents
```

Crash and file relay helpers:

```sh
ios -u <UDID> crash ls
ios -u <UDID> file --crash ls /
ios -u <UDID> file-relay Network --output network-relay.zip
```

Comparable upstream workflows:

- go-ios: `ios fsync ...`, `ios crash ls`, `ios crash cp`.
- pymobiledevice3: `pymobiledevice3 afc ...`,
  `pymobiledevice3 crash pull ...`.

## Applications and test automation

```sh
ios -u <UDID> apps list
ios -u <UDID> apps show com.example.app
ios -u <UDID> apps install ./Example.ipa
ios -u <UDID> apps uninstall com.example.app
ios -u <UDID> apps launch com.example.app
ios -u <UDID> apps processes
ios -u <UDID> apps kill <PID>
ios -u <UDID> apps roots
ios -u <UDID> apps spawn /usr/bin/log -- stream --style json
ios -u <UDID> apps icons com.example.app --output-dir ./icons
ios -u <UDID> apps monitor <PID> --timeout-secs 30
ios -u <UDID> runtest ./Build/Products/Example.xctestrun
ios -u <UDID> runtest ./Build/Products/Example.xctestrun --configuration UITests --test-target com.example.Runner --wait
ios -u <UDID> runwda --help
ios wda status --base-url http://127.0.0.1:8100
ios -u <UDID> wda --device-port 8100 status
ios -u <UDID> wda --device-port 8100 session --bundle-id com.example.Aut
```

`apps processes`, `apps launch`, `apps roots`, `apps spawn`, `apps icons`,
`apps monitor`, and related process-control commands use newer app service
paths and are mainly intended for iOS versions that expose those services
through CoreDevice/RSD.

`runtest` chooses the XCTest transport by iOS generation: iOS 17+ uses Remote
Service Discovery, iOS 14-16 uses the secure lockdown testmanager service, and
older versions use the legacy lockdown service. `wda --device-port` talks to a
WDA listener directly through usbmux, so it does not require a local `forward`
process when the runner is already listening on the device.

Comparable upstream workflows:

- go-ios: `ios apps`, `ios install`, `ios launch`, `ios kill`, `ios runtest`,
  `ios runwda`.
- pymobiledevice3: `pymobiledevice3 apps ...` and developer DVT launch/kill
  commands.

## Logs, diagnostics, and packet capture

```sh
ios -u <UDID> syslog
ios -u <UDID> diagnostics list
ios -u <UDID> diagnostics reboot
ios -u <UDID> os-trace ps
ios -u <UDID> pcap --output device.pcap
ios -u <UDID> notify wait com.apple.mobile.lockdown.host_attached
```

Use a test device for commands that restart the device, change state, or collect
large streams.

## Developer services

```sh
ios -u <UDID> ddi status
ios -u <UDID> ddi mount --path /path/to/DeveloperDiskImage.dmg
ios -u <UDID> instruments ps
ios -u <UDID> instruments cpu
ios -u <UDID> instruments sysmon-process <PID>
ios -u <UDID> instruments launch com.example.app
ios -u <UDID> instruments kill <PID>
ios -u <UDID> debugserver --help
ios -u <UDID> debug --help
ios -u <UDID> symbols list
ios -u <UDID> accessibility-audit capabilities
ios -u <UDID> webinspector opened-tabs
```

Many developer services require Developer Mode, a mounted Developer Disk Image,
or the CoreDevice tunnel path on newer iOS versions.

Comparable upstream workflows:

- go-ios: `ios image ...`, `ios instruments ...`, `ios debug ...`, `ios ax ...`.
- pymobiledevice3: `pymobiledevice3 mounter ...`,
  `pymobiledevice3 developer dvt ...`, `pymobiledevice3 webinspector ...`.

## iOS 17+ tunnel, RSD, and forwarding

```sh
ios -u <UDID> tunnel start --userspace
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
ios tunnel list
ios -u <UDID> rsd services
ios -u <UDID> forward 1234 62078 --once
```

Userspace tunnels expose a local TCP proxy. Kernel TUN mode may require
administrator or root privileges. See [tunnel.md](tunnel.md) for details.

Comparable upstream workflows:

- go-ios: `ios tunnel start`, `ios tunnel ls`, `ios rsd ls`, `ios forward`.
- pymobiledevice3: RemoteXPC/tunnel workflows and
  `pymobiledevice3 usbmux forward`.

## Management, profiles, and supervision

```sh
ios -u <UDID> profiles list
ios -u <UDID> provisioning list
ios -u <UDID> httpproxy set proxy.example.com 8080 --p12 identity.p12
ios -u <UDID> httpproxy remove
ios prepare create-cert ./supervision
ios -u <UDID> prepare --cert-der ./supervision.der
ios -u <UDID> power-assert --timeout 10
ios -u <UDID> preboard create
ios -u <UDID> restore enter-recovery
ios -u <UDID> restore events --count 5 --timeout-secs 30
ios -u <UDID> erase --force
```

These commands can change persistent device state. Prefer a test device, inspect
`--help`, and confirm the expected iOS version and supervision state before
running them.

`restore events` is a read-only RestoreRemoteServices event consumer. It waits
for lifecycle messages such as progress, status, checkpoint, data request,
previous log, or restored-crash notifications; it does not start a restore by
itself.

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

Use lower-level modules when you need direct control over usbmux, lockdown
sessions, service startup, DTX, XPC, or tunnel setup.

## Related documents

- [cli-map.md](cli-map.md) maps `ios` commands to comparable go-ios and
  pymobiledevice3 command families.
- [features.md](features.md) explains feature flags for library users.
- [tunnel.md](tunnel.md) covers CoreDevice tunnel setup in more detail.
- [troubleshooting.md](troubleshooting.md) covers common host and device issues.
