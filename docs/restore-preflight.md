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
substitute for recovery USB/ramdisk, FDR, ticket personalization or re-enumeration.
See [repair scope and verification](validation/review-fixes-2026-10-10.md).

The [2026-10-26 review](validation/remaining-six-review-2026-10-26.md) removed an unproven normal-mode/empty-firmware CLI execution path. Saved TSS metadata correlation never sets signature verification or execution support to true. Current-device and IPSW version strings are observations, not a validated compatibility policy. Unsupported restored data ports/types and malformed terminal statuses fail explicitly; no unserved request is counted as completed.

The [follow-up review](validation/six-followup-review-2026-10-26.md) also rejects execution with a saved `--tss` file, before device discovery. The removed branch treated a preflight dictionary as firmware bytes and substituted literal bytes on failure; it could not implement a restore. Product-version ordering now rejects malformed versions and obvious Update downgrades. It does not prove same-version build compatibility, signed eligibility, nonce stability across recovery or Apple signature validity. The reference profile describes iPhone14,2/d63ap (13 Pro), not iPhone14,3 (13 Pro Max), and is fixture metadata rather than an enabled execution profile.

## Selected OS extraction and ASR transfer

`IpswPreflight::extract_system_image` extracts only the selected identity's
`Manifest.OS.Info.Path` to a caller-owned writer, using 64 KiB chunks with size,
ZIP CRC and SHA256 observations under a cooperative deadline/cancellation.
Failure leaves partial output which the caller must discard. It does not load
the OS image into the smaller updater-component payload limit.

`restore::asr::send_image` owns an already-connected ASR socket and a seekable
image: unframed XML Initiate/offer, OOB range reads, Payload and 128 KiB chunks,
with SHA-1 per chunk when negotiated. `RestoredClient::send_system_image` serves
only the requested SystemImageData/DataPort after StartRestore; connection and
transfer inherit the original total clock and cancellation. Failure/caller Drop
makes the owner terminal. An ASR receipt proves local transfer, never flashing
completion; it cannot convert an arbitrary Status=0 into successful recovery.

Full Update still lacks proven recovery/ramdisk and USB/ECID reidentification,
nonce-aware TSS/IMG4/TBM/nonce-slot personalization, FDR and the complete async
component/data-port dispatcher. CLI --force and saved --tss remain rejected before
discovery. iPhone14,2/d63ap, board 0x0c/chip 0x8110, iOS17.5.1/21F90 is reference
fixture metadata only. No offline IPSW is currently available for that profile;
actual TSS, device transitions and recovery remain NOT_RUN. The missing executor
is a development gap independent of that file. See the
[current report](validation/production-completion-2026-10-26.md).


The experimental `run_restored_update_loop_with_system_image` now dispatches a
synchronous SystemImageData request through that ASR path and resumes the control
loop after transfer. AsyncDataRequest still needs an independent transport/task
owner and is explicitly rejected. This incomplete executor rejects Status=0 as
complete Update evidence; the older firmware-only helper retains its historical
protocol-loop result and does not certify recovery. See the
[development increment](validation/remaining-development-2026-10-27.md).
