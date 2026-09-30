# IPSW device-side preflight

`ios-core::restore::preflight::IpswPreflight` opens an IPSW ZIP and selects one
`BuildManifest.plist` identity from exact `ApBoardID`, `ApChipID`,
`Info.DeviceClass`, and `Info.RestoreBehavior` values. For Update and Erase it
prefers the corresponding standard IPSW variant; ambiguous matches fail. The
caller must obtain these selector values from a trusted device query. No
chip-only fallback is used for restore identity selection.

The reader limits the manifest to 8 MiB, the archive to 100,000 entries, and
each requested firmware member to 64 MiB. Firmware paths must be relative and
cannot contain `.` or `..` components. Duplicate ZIP names and missing members
fail. `payload_for_updater("Rose")` (or another supported updater) builds the
existing device-side preflight payload. The corresponding
`RestoreServiceClient::get_device_side_preflight_info_from_ipsw` method sends it
through a single-use service connection. Open a fresh restore service connection
for each updater; affected devices abort after a second preflight request.

This API only asks the normal-mode service to generate preflight information.
It does not start recovery, flashing, or firmware transfer. There is no
`preflight-requests` CLI yet: the supported iOS version boundary and the exact
normal-mode queries for board ID, chip ID, and hardware model are not established
by a device trace. A future CLI needs a redacted capture of those fields on a
supported device, a version rejection fixture, and a fake-service test that
verifies the selected identity and each updater request before it is exposed.
The existing `ios restore preflight-info` metadata query remains available.
