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
It does not start recovery or flashing. `ios --udid UDID restore preflight-requests
--ipsw PACKAGE` queries board/chip/model, selects the identity and opens a fresh
normal-mode service per updater. Connect and request share each updater's clock;
the CLI also applies a total deadline covering its connection and whole collection.
Partial results are emitted, and a failed updater causes nonzero exit. A supported
version/device trace and a version-rejection fixture remain missing; the entrypoint
is experimental and does not certify an iOS version range.

`restore update --dry-run` is a read-only *device* inspection, including nonce
queries, not an offline simulation. It reads all selected Manifest component paths
to EOF with ZIP CRC/size/path/aggregate checks and SHA256 inventory. Saved TSS input
must include original Request/Response metadata matching ECID, board/chip and AP/SEP
nonces; raw ticket bytes cannot establish that association. It does not verify
Apple ticket signatures or IMG4 personalization, and says so in its output.

Actual `restore update --force` currently rejects before connecting, because no
complete proven Update path is implemented. A distinct `restore::restored::RestoredClient`
now implements bounded already-connected restored plist negotiation/ECID checks,
StartRestore and real synchronous firmware responses; its peer fixtures do not
substitute for recovery USB/ramdisk, ASR, FDR, ticket personalization or re-enumeration.
See [repair scope and verification](validation/review-fixes-2026-10-10.md).
