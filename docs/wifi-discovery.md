# Wi-Fi discovery and existing trust records

`discover_paired_mobdev2_devices()` now recognizes `_apple-mobdev2._tcp` services
using both recorded Wi-Fi MAC addresses and the advertisement's `authTag` /
`authTag#N` fields. A mobdev2 tag identifies the paired host, so multiple devices
can legitimately match the same tag. A private MAC is resolved by offering the
matching existing records to lockdown, reading `UniqueDeviceID` inside the
established session, and accepting only the record whose device identifier agrees.
The probe sends `StopSession` and closes the connection. It never sends a Pair
request, shows a trust prompt, installs software, or starts a tunnel.

Lockdown discovery reads the platform's existing lockdown directory:
`/var/lib/lockdown` on Linux, `/var/db/lockdown` on macOS, and
`%ALLUSERSPROFILE%/Apple/Lockdown` on Windows. Missing, unreadable and malformed
records are isolated. No extra credential locations are searched. If that
directory is not readable, this API returns no paired mobdev2 devices; enumeration
of records held only by usbmuxd is not included yet.

`connect_remote_pairing_tunnel()` matches `_remotepairing._tcp` advertisements
offline using the device's saved `peer_alt_irk` before opening a connection.
It uses the normal `~/.ios-rs` and `~/.pymobiledevice3` remote-pairing stores
(the normal application-data directory replaces `~/.ios-rs` on Windows).
An empty device identifier selects a single recognized device. Multiple devices
or duplicate discovery keys are rejected as ambiguous; an explicit device or
host filter selects the desired device. The advertised opaque `identifier` is
never assumed to be a UDID.

New explicit pair operations retain the optional device `altIRK` received in the
authenticated M6 info payload. Remote records use plist `<data>` for their byte
fields; older array-encoded Rust records remain readable. Records predating
`peer_alt_irk` still work through direct RSD, but do not identify RemotePairing
Wi-Fi advertisements. There is no automatic re-pairing or record migration.

The mobdev2 resolver considers at most 16 candidate records per advertisement,
32 probes total, three seconds per probe and twelve seconds total after the
Bonjour browse. RemotePairing connection attempts have a ten-second limit each
and a thirty-second overall limit after discovery. Record files are limited to
one MiB; discovery uses at most 128 records and 256 service instances. These
budgets can omit devices on unusually large or slow networks.

Fixed cryptographic vectors, synthetic advertisements, candidate-selection
probes, malformed-record isolation and persistence compatibility are covered by
host tests. Actual TLS lockdown acceptance, private-address rotation, scoped IPv6
connectivity and RemotePairing tunnel establishment still need device validation.
