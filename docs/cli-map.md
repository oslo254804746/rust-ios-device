# CLI map

This document maps the `ios` CLI in this workspace to comparable command
families from go-ios and pymobiledevice3. It is intended for users who already
know one of those tools and want to find the nearest rust-ios-device command.

The mapping is functional rather than exact. Command names, flags, output
schemas, service routing, and iOS-version support can differ.

## Global conventions

| Area | rust-ios-device | go-ios | pymobiledevice3 |
| --- | --- | --- | --- |
| Binary | `ios` | `ios` | `pymobiledevice3` |
| Select device | `-u <UDID>` or `IOS_UDID=<UDID>` | `--udid=<UDID>` | commonly `--udid <UDID>` or service-provider options |
| JSON output | Default for most structured commands | Default | Varies by command |
| Human output | `--no-json` where supported | `--nojson` | Varies by command |
| Verbosity | `-v`, repeat for more detail | `-v`, `--trace` | logging options vary |

## Command families

| Task | rust-ios-device command | Comparable go-ios command | Comparable pymobiledevice3 command |
| --- | --- | --- | --- |
| List devices | `ios list` | `ios list` | `pymobiledevice3 usbmux list` |
| Watch attach/detach | `ios listen` | `ios listen` | usbmux/bonjour-oriented workflows |
| Bonjour discovery | `ios discover mobdev2` | tunnel/RSD discovery flags and helpers | `pymobiledevice3 bonjour rsd` |
| Pair device | `ios pair` | `ios pair` | lockdown pairing flows |
| Show pair record | `ios pair show-record` | `ios readpair` | lockdown pair-record helpers |
| Lockdown query | `ios lockdown get --key ProductVersion` | `ios lockdown get ProductVersion` | `pymobiledevice3 lockdown ...` |
| Device info | `ios info` | `ios info` | lockdown and diagnostics commands |
| Disk usage | `ios diskspace` | `ios diskspace` | lockdown/diagnostics equivalents |
| MobileGestalt | `ios mobilegestalt KEY...` | `ios mobilegestalt KEY...` | diagnostics/mobilegestalt-oriented helpers |
| Battery summary | `ios batterycheck` | `ios batterycheck` | diagnostics battery workflows |
| Battery registry | `ios batteryregistry` | `ios batteryregistry` | diagnostics IORegistry workflows |
| AFC file access | `ios file ls /`, `ios file pull ...`, `ios file push ...` | `ios fsync ...` | `pymobiledevice3 afc ...` |
| App container files | `ios file --app <BUNDLE_ID> ...` | app-container fsync style workflows | AFC/house-arrest workflows |
| Crash reports | `ios crash list`, `ios file --crash ...` | `ios crash ls`, `ios crash cp` | `pymobiledevice3 crash pull ...` |
| File relay | `ios file-relay <SOURCE>` | file relay service workflows | file relay service workflows |
| List apps | `ios apps list` | `ios apps` | `pymobiledevice3 apps list` |
| Install app | `ios apps install PATH` | `ios install --path=PATH` | `pymobiledevice3 apps install PATH` |
| Uninstall app | `ios apps uninstall BUNDLE_ID` | app uninstall workflows | `pymobiledevice3 apps uninstall ...` |
| Launch app | `ios apps launch BUNDLE_ID` or `ios instruments launch BUNDLE_ID` | `ios launch BUNDLE_ID` | `pymobiledevice3 developer dvt launch ...` |
| Kill process | `ios apps kill PID`, `ios instruments kill PID`, `ios memlimitoff PID` | `ios kill ...`, `ios memlimitoff ...` | `pymobiledevice3 developer dvt kill ...` |
| Run XCTest | `ios runtest FILE.xctestrun [--configuration NAME --test-target TARGET --wait]` | `ios runtest ...`, `ios runxctest ...` | developer DVT/XCTest workflows |
| Run WebDriverAgent | `ios runwda ...`, `ios wda status/source/session/...` | `ios runwda ...` | WDA/developer workflows |
| Syslog | `ios syslog` | `ios syslog` | `pymobiledevice3 syslog live` |
| Diagnostics | `ios diagnostics ...` | `ios diagnostics ...` | `pymobiledevice3 diagnostics ...` |
| Restart or restore mode | `ios diagnostics reboot`, `ios restore enter-recovery` | `ios reboot`, restore helpers | `pymobiledevice3 diagnostics restart`, `pymobiledevice3 restore ...` |
| Packet capture | `ios pcap --output device.pcap` | `ios pcap ...` | `pymobiledevice3 pcap ...` |
| OS trace | `ios os-trace ps`, `ios instruments trace` | `ios sysmontap`, trace-related tools | `pymobiledevice3 developer dvt oslog` |
| Developer Disk Image | `ios ddi status`, `ios ddi mount ...` | `ios image list`, `ios image mount`, `ios image auto` | `pymobiledevice3 mounter auto-mount` |
| Instruments process list | `ios instruments ps` | `ios ps`, `ios instruments ...` | `pymobiledevice3 developer dvt sysmon ...` |
| Instruments CPU/memory | `ios instruments cpu`, `ios instruments sysmon-process ...` | `ios sysmontap` | `pymobiledevice3 developer dvt sysmon ...` |
| Network and GPU metrics | `ios instruments network`, `ios instruments gpu` | instruments/sysmontap workflows | developer DVT metrics workflows |
| Debugserver | `ios debugserver ...`, `ios debug ...` | `ios debug ...` | debugserver developer workflows |
| Accessibility audit | `ios accessibility-audit ...` | `ios ax ...`, accessibility toggles | accessibilityaudit service workflows |
| WebInspector | `ios webinspector ...` | WebInspector-related workflows | `pymobiledevice3 webinspector ...` |
| Symbols | `ios symbols list`, `ios symbols pull ...` | symbol fetch workflows | `dtfetchsymbols` / remote symbols workflows |
| Tunnel start | `ios tunnel start --userspace` | `ios tunnel start` | iOS 17+ tunnel and RemoteXPC workflows |
| Tunnel manager | `ios tunnel serve --userspace ...` | go-ios tunnel HTTP manager | tunneld/remote workflows |
| RSD services | `ios rsd services` | `ios rsd ls` | Remote Service Discovery workflows |
| Port forwarding | `ios forward HOST_PORT DEVICE_PORT` | `ios forward HOST_PORT TARGET_PORT` | `pymobiledevice3 usbmux forward HOST_PORT DEVICE_PORT` |
| Profiles | `ios profiles list` | `ios profile list`, `ios profile add`, `ios profile remove` | `pymobiledevice3 profile ...` |
| Provisioning profiles | `ios provisioning list` | provisioning/profile workflows | `pymobiledevice3 provision ...` |
| Supervision prep | `ios prepare ...` | `ios prepare ...` | mobile configuration and supervision workflows |
| HTTP proxy profile | `ios httpproxy set ...`, `ios httpproxy remove` | `ios httpproxy ...` | profile/mobileconfig workflows |
| Erase | `ios erase --force` | `ios erase --force` | restore/mobile configuration workflows |
| Preboard | `ios preboard ...` | prepare/preboard-style workflows | preboard service workflows |
| Power assertion | `ios power-assert --timeout 10` | power assertion workflows | `pymobiledevice3 power-assertion ...` |
| Companion devices | `ios companion list` | companion-related workflows | `pymobiledevice3 companion_proxy ...` |

## Notes for migration

- `ios file` intentionally groups AFC, House Arrest, and crash-report file access
  behind one command with mode flags. go-ios users may be used to `fsync`; pymobiledevice3
  users may be used to separate AFC service commands.
- `ios apps` contains both classic installation-proxy operations and newer iOS 17+
  appservice process controls. If a command fails on an older device, check whether
  an `instruments` command or a classic lockdown service is the correct path.
- `ios tunnel` and `ios rsd` are the entry points for CoreDevice/RSD workflows.
  These are the closest equivalents to the iOS 17+ tunnel flows in go-ios and
  pymobiledevice3, but the local tunnel manager API and userspace proxy protocol
  are rust-ios-device specific.
- Management commands such as `erase`, `prepare`, `httpproxy`, `profiles`,
  `restore`, and `preboard` can change persistent device state. Use a test device
  and inspect command-specific help before running them.

## Source orientation

The CLI front end lives in `crates/ios-cli`. Service implementations and protocol
layers live in `crates/ios-core`. The current command list can always be checked
with:

```sh
cargo run -p ios-cli -- --help
```
