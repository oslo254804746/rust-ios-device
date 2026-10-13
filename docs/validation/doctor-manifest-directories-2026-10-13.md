# F01 target personalization manifests and host directory observations

2026-10-13 implementation record. This page records development scope and
fixture design and actual root validation. Doctor fixtures: **60 PASS, 0 FAIL,
1 original ignored**; the first run was 58 PASS/2 FAIL because two exact order
lists still described seven checks. Both lists now append the eighth check and
also verify its independent original failure/empty metadata; original assertions
remain. The root agent is the sole Rust/Cargo/maturin validator. No real
device discovery or device actions were executed for this work.

## Production change

An explicit doctor target appends `ddi_manifests` after the existing seven
device checks. It owns a fresh existing-trust session under one checked absolute
deadline spanning enumeration, local pair-record load, lockdown/TLS, service
startup, `CopyDevices`, and every manifest request/reply. Host-only doctor never
opens this target session. No pairing, settings, download, mount, tunnel creation
or developer operation is added by this manifest check.

Protocol fields come from the already implemented `ImageMounterClient`
`copy_devices` and `query_personalization_manifest` wire in
`crates/ios-core/src/services/imagemounter/protocol.rs` and the existing
`ios ddi personalization-manifests` command. The new observer preserves absent
EntryList/entry metadata rather than using the legacy helper's empty defaults.
It selects only CopyDevices entries explicitly reporting `DiskImageType =
Personalized` and `PersonalizedImageType = DeveloperDiskImage`. Each nonempty
reported data signature is sent using the existing QueryPersonalizationManifest
request's Command/PersonalizedImageType/ImageType/ImageSignature fields. A
nonempty response data ImageSignature is an opaque returned manifest/ticket
observation. It is **not** a local BuildManifest.plist, parsed IMG4 trust chain,
Apple cryptographic signature verification or evidence of all developer services.

The entire mounted-entry selection is admitted before manifest queries: at most
128 entries, 32 queries and 1 MiB total retained query signatures. The Core
lockdown decoder bounds each wire frame to 4 MiB before allocation. The observer
also admits each manifest to at most 1 MiB and total accepted manifest payloads
to 4 MiB. Responses are dropped after counting; report metadata contains only
entry/query/byte counts and `complete`, never raw types, signatures, tickets,
identifiers, paths or response bodies. A missing/empty response leaves Unknown,
malformed/over-limit data fails as protocol, and partial failures retain acquired
counts plus the original native/lockdown error. These payload limits do not
claim to bound plist-parser temporary allocations, CPU or allocator capacity.

Host `pairing_directory` derives its path from the same public path resolver as
PairRecord::load. It requires an absolute resolved location and enumerates at
most 4096 entries without reading file contents, testing write access or creating
anything. Counts include regular plist files only; readable directory metadata
does not establish selected-target trust or whether a particular pair file can
be loaded. Paths and names are not emitted. This diagnoses the Windows relative
resolver result when ALLUSERSPROFILE is absent without reading the working tree.

Linux USB now observes at most 4096 sysfs entries and reads at most 65 bytes per
idVendor. Missing interface idVendor files are skipped; directory/read failures,
malformed values and truncated enumeration cannot produce an operational/OK
claim. Windows counts require typed, mutually consistent total/apple/failed
values. macOS IOUSB uses exact numeric vendor values instead of substring
matching (`14520` is not Apple VID `1452`); malformed UTF-8/vendor fields stay
unknown. Native IO failures retain their cause and expose safe optional host
`state`/`error_class`, including missing metadata, permission denied and
unreachable. Unsuccessful host-command exits do not guess permission from an
exit code. A host Available state describes the successful observation itself,
while status/description preserve the limit on capability conclusions.

Pairing/DDI/sysfs work runs in blocking workers rather than synchronously blocking
the async runtime. The outer deadline/cancellation stops accepting late results;
it cannot physically interrupt a stalled OS filesystem call. Runtime exit may
still wait for that worker. This physical cancellation gap is not closed here.

Schema 1 host and schema 2 target remain; additions are a host check, optional
host state/error fields, and the eighth target check. Existing target checks keep
their order. The raw diagnostic fixture's exact set and failure-category
assertions are updated to all eight implemented checks, including the three
previously added mounted/usability/manifest checks. The capability registry scope
is corrected without assigning a new PASS or maturity upgrade.

## Fixture gates for the root agent

All new device fixtures use owned duplex/TLS sessions and the production
observer, supported wire fields and strict original-peer closure. They cover
successful multi-signature queries with opaque count output, missing/empty and
malformed entry/payload metadata, count/byte admission before requests, partial
error counts and original error identity, StartService permission/service-missing
categories, shared whole-probe timeout and cancellation. The existing complete
assessment fixture now verifies all six fresh existing-trust sessions and exact
port-call order. Host fixtures create owned temporary directories; they cover
names-only enumeration, missing/non-directory/error causes, Linux vendor values,
Windows count consistency and exact macOS VID parsing. Synthetic values exercise
these known adapters; they do not establish previously unknown Apple field or
device behavior.

Root gates: format; CLI all-features/all-targets check; targeted
`cargo test -p ios-cli --all-features --locked cmd::doctor::`; existing diagnostic
tooling unit tests/AST/JSON registry validation; strict CLI Clippy and affected
CLI regression suite. Keep source frozen throughout each compiled gate. Do not
run ordinary CLI doctor/raw diagnostic captures while real discovery remains
unauthorized: even a usbmuxd trap does not stub the platform USB probe. Fixture
tests themselves do not invoke real NativeSource/device enumeration. Record
actual results and hashes centrally; this note does not predeclare PASS.

Python diagnostic tooling is 7 PASS/1 original Unix-only skip. Its initial run
rejected a new SpringBoard fixture's derived Debug; fixed-description Debug
replaced that derive without changing the scanner or runtime assertions. Python
AST, JSON parse and diff checks pass. Strict Clippy/build/registry results are
recorded in the external frozen-source summary after their actual execution.

Final strict CLI all-feature/all-target Clippy, CLI build and fmt/diff passed.
Capability inventory checked 119 records against actual CLI help without
changing maturity. Shared read-only manager smoke passed with zero Source
connections; host runner was 5 PASS/22 NOT_RUN, real_device_executed=false.
No ordinary production doctor capture was run because its platform USB
observation would perform actual discovery. The production observer's new
success/failure wire behavior is covered by the above isolated fixtures.

## Full original F01 remaining scope

The original F01 is not closed by this independently reviewable addition.
Complete local DDI BuildManifest-to-target board/chip/security-domain matching
and a validated local component/digest/signature evidence path still need
production wiring and documented protocol/filesystem admission. Returned
personalization manifests do not establish Apple IMG4 trust-chain validity.
There is no local BuildManifest target selector in doctor. The current DDI
directory scanner discovers only bounded regular DMG/signature companions;
personalized trustcache/component inventory and target compatibility remain
unprobed.

Physical USB/USBMUX target association, representative existing-trust/Developer
Mode/mounted/usability states, iOS/firmware/transport support, physical RSD routes,
and macOS remoted before/after runtime evidence remain NOT_RUN. Windows/macOS
command-output adapters and Linux sysfs access have only fixture coverage in
this work; snapshots/counts do not prove selected-device identity or continuous
liveness. The host platform_route probe still observes the direct selected IPv6
endpoint independently of an explicitly chosen loopback userspace proxy; only
the device existing-RSD check establishes that proxy's selected target route.
Legacy handshake-only raw DTX remains limited to negotiated TLS 1.2; TLS 1.3 is
explicitly unsupported. Other developer services and unimplemented transports
must remain Unknown/unsupported. Full filesystem/process physical cancellation,
resource release across all supported hosts and final Q01 real-support evidence
remain original acceptance work, not automatic consequences of these fixtures.
