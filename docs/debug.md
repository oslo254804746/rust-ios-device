# Debug with LLDB and debugserver

`ios --udid <UDID> debug <bundle-id> [--local-app <path-to-local.app>]` opens the device's iOS 17–27.2 remote `com.apple.internal.dt.remote.debugproxy` service through a userspace tunnel. It resolves the installed app path, waits for the userspace proxy's remote-connect acknowledgement, prints LLDB commands, and stays in the foreground while one local LLDB client uses the temporary `127.0.0.1` port. Keep the command running; after the client disconnects the forwarding session ends. Ctrl+C cancels and closes the session. The endpoint accepts a local connection without authentication, so use it only on a trusted host.

```sh
ios --udid <UDID> debug com.example.app --local-app /path/to/local/application.app
ios --udid <UDID> debugserver send qSupported
```

For low-level GDB Remote Serial Protocol probes, `ios --udid <UDID> debugserver send <packet>` first attempts the classic Lockdown debugserver service (`com.apple.debugserver.DVTSecureSocketProxy` or `com.apple.debugserver`). On iOS 17–27.2 devices where Lockdown returns `InvalidService` (`ServiceMissing`), `debugserver send` automatically falls back to CoreDevice RSD `com.apple.internal.dt.remote.debugproxy` over the userspace tunnel and exchanges the `$packet#checksum` frame directly.

Paste the lines printed by `ios debug` into a compatible LLDB session. The generated `process launch` starts the selected app when executed; it is not executed by `ios`. Supply a local app bundle matching the installed app and debug symbols. If `--local-app` is omitted, the output contains `/path/to/local/application.app`, which must be replaced before use. Paths containing control characters are rejected; device paths containing a backslash are also rejected because Windows LLDB normalizes it as a separator. A failed RSD lookup or missing remote debugproxy is reported before app lookup; Developer Mode and a mounted Developer Disk Image (or Cryptex DDI on iOS 27+) are required for the device to expose `com.apple.internal.dt.remote.debugproxy`.

Both `ios debug <bundle-id>` (LLDB script generation and userspace proxy handshake) and `ios debugserver send qSupported` (RSD `debugproxy` fallback returning the full feature string such as `PacketSize=20000;QStartNoAckMode+;...`) have been verified on physical iOS 27.2 devices with Cryptex DDI `v27.1.5228.8` mounted. See the [capability inventory](capabilities.md) for recorded test scopes.
