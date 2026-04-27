# Feature flags

`ios-core` is published with no default service features. A minimal dependency can list devices, talk to usbmuxd/lockdown, and use the high-level connection types without pulling every service client into downstream builds.

Enable only the services your application needs:

```toml
[dependencies]
ios-core = { version = "0.1.4", features = ["afc", "syslog"] }
```

For tools that intentionally expose a broad surface, use grouped features:

```toml
ios-core = { version = "0.1.4", features = ["classic", "developer"] }
```

## Groups

| Feature | Purpose |
| --- | --- |
| `classic` | Common lockdown/usbmux services used across many iOS versions. |
| `developer` | DTX, Instruments, debugserver, WebInspector, image mounting, and related developer workflows. |
| `management` | Device management, supervision/preparation, restore, power assertion, and companion-device helpers. |
| `ios17` | CoreDevice/RSD-oriented services and tunnel workflows used primarily by iOS 17+ devices. |
| `full` | Everything exposed by `ios-core`; intended for the CLI and integration testing. |

## Service features

Most service modules are available as one feature per module, including `afc`, `apps`, `syslog`, `screenshot`, `dtx`, `instruments`, `testmanager`, `accessibility_audit`, `debugserver`, `imagemounter`, `pcap`, `webinspector`, `fileservice`, and `deviceinfo`.

Some features add heavier optional dependencies only when enabled:

| Feature | Extra dependency surface |
| --- | --- |
| `apps` | IPA/Zip parsing and CRC support. |
| `imagemounter` | HTTP downloads plus Zip handling for Developer Disk Images. |
| `dtx`, `instruments`, `testmanager`, `accessibility_audit`, `dproxy` | DTX codec support. |

The `ios-cli` crate enables `ios-core/full` because the binary exposes many commands. Library users should prefer a narrower feature list.
