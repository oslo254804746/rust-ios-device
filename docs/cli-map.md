# CLI map

This document maps the `ios` CLI in this workspace to comparable command
families from go-ios and pymobiledevice3. It is intended for users who already
know one of those tools and want to find the nearest rust-ios-device command.

The mapping is functional rather than exact. Command names, flags, output
schemas, service routing, and iOS-version support can differ.

## Global conventions

Automatic errors follow the [diagnostic data contract](diagnostic-contract.md):
command phase, error category and process-local correlation, without raw source
chains or credential-bearing environment values. Runtime failures exit 1;
argument failures exit 2 and direct the caller to command help.

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
| Host diagnostics | `ios doctor [--mdns]` | host/tunnel diagnostics | `pymobiledevice3 doctor` |
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
| AFC file access | `ios file ls /`, `pull`, `push`, `stat`, `mkdir`, `mv`, `rm [--recursive]`, `tree`, `device-info`, `connection-info`, `hash [--offset] [--length]`, `pread`, `pwrite`, `truncate [--by-fd]`, `set-time`, `dir-size` | `ios fsync ...` | `pymobiledevice3 afc ...` |
| App container files | `ios file --app <BUNDLE_ID> [--documents] ...` | app-container fsync style workflows | AFC/house-arrest workflows |
| CoreDevice FileService | `ios file --coredevice --domain <DOMAIN> [--identifier <BUNDLE_ID>] ls|mkdir|push|pull|mv|rm [--recursive]` (also referred to as `ios fileservice` workflows) | CoreDevice file service helpers | `pymobiledevice3 developer core-device file-service ...` |
| Crash reports | `ios crash ls|pull|parse|parse-latest|flush|watch|clear`, `ios file --crash ...` | `ios crash ls`, `ios crash cp` | `pymobiledevice3 crash pull|parse|parse-latest|flush|watch ...` |
| File relay | `ios file-relay <SOURCE>` | file relay service workflows | file relay service workflows |
| List apps | `ios apps list [--coredevice]` | `ios apps` | `pymobiledevice3 apps list` |
| App icons | `ios apps icons BUNDLE_ID` (alias: `apps icon`) | app icon workflows | `pymobiledevice3 developer core-device icon ...` |
| Install app | `ios apps install PATH` | `ios install --path=PATH` | `pymobiledevice3 apps install PATH` |
| Uninstall app | `ios apps uninstall BUNDLE_ID` | app uninstall workflows | `pymobiledevice3 apps uninstall ...` |
| Install record | `ios apps install-record BUNDLE_ID [--timeout-secs N]` | LaunchServices install-coordination query | `pymobiledevice3 apps install-record ...` (iOS 17+ RSD) |
| Launch app | `ios apps launch BUNDLE_ID` or `ios instruments launch BUNDLE_ID` | `ios launch BUNDLE_ID` | `pymobiledevice3 developer dvt launch ...` |
| Kill/process controls | `ios apps kill PID`, `ios apps pkill PATTERN`, `ios instruments kill PID`, `ios memlimitoff PID` or `ios memlimitoff --process NAME` | `ios kill ...`, `ios memlimitoff ...` | `pymobiledevice3 developer dvt kill ...` |
| Device pasteboard | `ios pasteboard get|set|watch` (Darwin notifications and fresh PULLs); experimental `resolve|export --experimental` and `watch --autonotify --experimental` (output redacted unless `--show-data`) | `ios pasteboard get`, `ios pasteboard set [TEXT]` | `pymobiledevice3 developer core-device paste`, `copy [TEXT]`, pasteboard monitor |
| Run XCTest | `ios runtest FILE.xctestrun [...]` or `ios runxctest --test-runner-bundle-id BUNDLE --xctest-config BUNDLE.xctest [--bundle-id APP ... --wait --junit-output PATH]` | `ios runtest ...`, `ios runxctest ...` | developer DVT/XCTest workflows; runner must already be installed and signed |
| Run WebDriverAgent | `ios runwda ...`, `ios wda status/source/session/...` | `ios runwda ...` | WDA/developer workflows |
| Syslog | `ios syslog` | `ios syslog` | `pymobiledevice3 syslog live` |
| Screenshot | `ios screenshot [--output PATH]` | `ios screenshot ...` | `pymobiledevice3 developer dvt screenshot` / CoreDevice screen capture |
| Web / VNC Viewer | `ios viewer web|vnc|clipboard [--share-clipboard] [--local-unauthenticated]` | screen/HID viewer workflows | screen capture, HID, and clipboard tools |
| Diagnostics | `ios diagnostics ...`, `ios diagnostics sysdiagnose` | `ios diagnostics ...` | `pymobiledevice3 diagnostics ...`, CoreDevice sysdiagnose workflows |
| MobileBackup2 | `ios backup create|restore|info|list`, `backup list-local|unback|extract` (host-side), `backup unback-device|extract-device`, `backup encryption` | backup/device-link workflows, including host-side manifest inspection/extraction | `pymobiledevice3 backup ...`, local pyiosbackup extraction |
| Restart or restore mode | `ios diagnostics reboot`, `ios diagnostics shutdown`, `ios restore enter-recovery` | `ios reboot`, restore helpers | `pymobiledevice3 diagnostics restart`, `pymobiledevice3 restore ...` |
| Packet capture | `ios pcap [--interface/-i <INTERFACE>] [--pid <PID>] [--process <PREFIX>] --output device.pcap` | `ios pcap ...` | `pymobiledevice3 pcap ...` (resolves iOS 27.2 unit-suffixed interfaces such as `en0`, `pdp_ip0`) |
| Device IP discovery | `ios ip` (packet-derived IPv4/IPv6) | `ios diagnostics` / `pcap` IP finder | `pymobiledevice3 pcap` workflows |
| Bluetooth HCI capture | `ios btlogger capture trace.pklg`, `ios btlogger capture --format pcapng trace.pcapng` | Bluetooth packet logging workflows | `pymobiledevice3 btlogger ...` |
| OS trace | `ios os-trace ps`, `ios os-trace stream`/`live`, `ios os-trace archive`/`collect`, `ios instruments trace` | `ios sysmontap`, trace-related tools | `pymobiledevice3 developer dvt oslog`, `syslog collect` |
| Developer Disk Image | `ios ddi auto|status|mount|unmount`, `ios ddi cryptex ...` (iOS 27+ Cryptex `v27.1.5228.8` auto-install) | `ios image list`, `ios image mount`, `ios image auto` | `pymobiledevice3 mounter auto-mount`, `pymobiledevice3 cryptex ...` |
| Instruments process list | `ios instruments ps` | `ios ps`, `ios instruments ...` | `pymobiledevice3 developer dvt sysmon ...` |
| Instruments CPU/memory | `ios instruments cpu`, `ios instruments sysmon-process ...` | `ios sysmontap` | `pymobiledevice3 developer dvt sysmon ...` |
| CoreDevice appearance/accessibility | `ios device-control configuration get|set <style|color-filter|text-size|reduce-motion|increase-contrast|show-borders|reduce-transparency|liquid-glass-opacity>` | `pymobiledevice3 developer core-device user-interface-style ...` and accessibility controls | iOS 17+ `com.apple.coredevice.configuration`; setters mutate device-wide UI state |
| CoreDevice orientation | `ios device-control orientation [left|right]` | `pymobiledevice3 developer core-device rotate [left|right]` | iOS 17+ `com.apple.coredevice.devicecontrol`; rotates the active device UI |
| CoreDevice HID input | `ios hid --confirm button|tap|swipe|touch|multitouch --points <id:x:y,...> [--hold-ms]|text|key [--shift]` | `pymobiledevice3 developer core-device` HID helpers | iOS 17+ Indigo/Universal HID; automatic Userspace Tunnel fallback, 500ms touch settle, 300ms keyboard settle |
| CoreDevice display media | `ios device-control display status|video|audio` (bounded encoded RTP capture; works over Userspace Tunnel on iOS 27+) | `pymobiledevice3 developer core-device display ...` | iOS 17+ `displayservice`; raw Annex-B HEVC or AAC-ELD access units |
| Network and GPU metrics | `ios instruments network`, `ios instruments gpu` | instruments/sysmontap workflows | developer DVT metrics workflows |
| Debugserver & LLDB | `ios debug <bundle_id> [--local-app <PATH>]`, `ios debugserver send <packet>` | `ios debug ...` | debugserver developer workflows (automatic fallback to CoreDevice RSD `com.apple.internal.dt.remote.debugproxy` on iOS 17–27.2) |
| Accessibility audit | `ios accessibility-audit ...` | `ios ax ...`, accessibility toggles | accessibilityaudit service workflows |
| WebInspector | `ios webinspector opened-tabs|eval|launch|js-shell|cdp|selenium` | WebInspector launch/evaluation and bridge workflows | `pymobiledevice3 webinspector ...` |
| Symbols | `ios symbols list`, `ios symbols pull ...` | symbol fetch workflows | `dtfetchsymbols` / remote symbols workflows |
| Tunnel start | `ios tunnel start --userspace` | `ios tunnel start` | iOS 17+ tunnel and RemoteXPC workflows |
| Tunnel manager | `ios tunnel serve --userspace ...` | go-ios tunnel HTTP manager | tunneld/remote workflows |
| RSD services | `ios rsd services --all --features` | `ios rsd ls` | Remote Service Discovery workflows |
| Port forwarding | `ios forward HOST_PORT DEVICE_PORT` | `ios forward HOST_PORT TARGET_PORT` | `pymobiledevice3 usbmux forward HOST_PORT DEVICE_PORT` |
| Profiles | `ios profiles list` | `ios profile list`, `ios profile add`, `ios profile remove` | `pymobiledevice3 profile ...` |
| Provisioning profiles | `ios provisioning list` | provisioning/profile workflows | `pymobiledevice3 provision ...` |
| Supervision prep | `ios prepare ...` | `ios prepare ...` | mobile configuration and supervision workflows |
| HTTP proxy profile | `ios httpproxy set ...`, `ios httpproxy remove` | `ios httpproxy ...` | profile/mobileconfig workflows |
| MDM passcode/security | `ios mdm fetch-unlock-token`, `ios mdm security-info`, `ios mdm passcode-present`, `ios mdm clear-passcode`, `ios mdm clear-screen-time-password` | `ios mdm ...` | MCInstall/profile management workflows |
| Managed Wi-Fi profile | `ios wifi install SSID --password ...`, `ios wifi remove SSID --force` | `ios wifi ...` | MCInstall/profile workflows |
| Erase | `ios erase --force` | `ios erase --force` | restore/mobile configuration workflows |
| Preboard | `ios preboard ...` | prepare/preboard-style workflows | preboard service workflows |
| Power assertion | `ios power-assert --timeout 10` | power assertion workflows | `pymobiledevice3 power-assertion ...` |
| Companion devices | `ios companion list|get|listen|forward|stop ...` | companion registry, event, and port-forward workflows | `pymobiledevice3 companion_proxy ...` |
| Activation | `ios activation state`, `ios activation session-info`, `ios activation info`, `ios activation activate [--record-input PATH] [--now]`, `ios activation deactivate --force`, `ios activation itunes-activate` | go-ios activation helpers | pymobiledevice3 activation workflows |
| AMFI developer mode | `ios amfi enable-developer-mode`, `ios amfi reveal-developer-mode` | `ios amfi ...` | `pymobiledevice3 amfi ...` |
| Heartbeat | `ios heartbeat` | heartbeat service workflows | lockdown heartbeat service |
| Notifications | `ios notify wait <EVENT>` | `ios notify ...` | `pymobiledevice3 notification ...` |
| SpringBoard | `ios springboard ...` | `ios springboard ...` | pymobiledevice3 springboard workflows |
| Traffic recording | `ios dproxy service <SERVICE>` | DTX proxy/recording workflows | developer DVT service inspection |
| IDAM | `ios idam get` | IDAM-related workflows | IDAM service workflows |

## Notes for migration

- `ios file` groups classic AFC (including extended opcodes `connection-info`,
  `hash`, `pread`, `pwrite`, `truncate`, `set-time`, and `dir-size`), House
  Arrest (`--app`), crash-report file access (`--crash`), and iOS 17+ CoreDevice
  `fileservice` (`--coredevice --domain <DOMAIN>`) behind one command.
- `ios apps` contains both classic installation-proxy operations and newer iOS 17+
  appservice process controls. If a command fails on an older device, check whether
  an `instruments` command or a classic lockdown service is the correct path.
- `ios tunnel` and `ios rsd` are the entry points for CoreDevice/RSD workflows.
  The userspace tunnel includes a 20ms ICMPv6 Echo Request Nudge and IPv6 UDP
  datagram forwarding so `device-control display`, `hid`, and `viewer` work
  without kernel TUN privileges.
- `ios debugserver send <packet>` automatically falls back from classic Lockdown
  debugserver to CoreDevice RSD `com.apple.internal.dt.remote.debugproxy` on
  iOS 17–27.2 when Lockdown reports `ServiceMissing`.
- Management commands such as `erase`, `prepare`, `httpproxy`, `profiles`,
  `restore`, and `preboard` can change persistent device state. Use a test device
  and inspect command-specific help before running them.
- Activation separates the device-side `com.apple.mobileactivationd` plist
  protocol from Apple's HTTPS endpoints. Online `activation activate` uses the
  official HTTPS endpoints by default; custom endpoints require
  `--unsafe-custom-server` and TLS verification is never disabled. Offline
  records are read from or written to owner-only files and activation payloads
  are redacted in diagnostic output. `deactivate` requires `--force`.
- Online activation waits for a fresh Tunnel1 nonce/session by default. `--now`
  skips that wait and sends one session probe, which is useful only when the
  caller controls nonce freshness; the operation timeout covers polling,
  network requests, and device application.

## Source orientation

The CLI front end lives in `crates/ios-cli`. Service implementations and protocol
layers live in `crates/ios-core`. The current command list can always be checked
with:

```sh
cargo run -p ios-cli -- --help
```
