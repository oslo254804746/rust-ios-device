# Explicit device doctor and manual tunnel admission

Accepted on 2026-10-08, Linux x86_64. Implementations:

- Manual selectors: `f0ec223c56c5918e5576a969ca2c8f9101cf364a`.
- Pair-record file admission: `82c6c5665c2121ba1649b28de615d7f1594b90e4`.
- Explicit device doctor: `71a59fc2152ad4dbdff26c08d5152864a078a815`.

The final combined code state is `71a59fc`. These are L01/L02 and F01 host
work items. Their main tasks retain all other original acceptance requirements.

## Delivered behavior

Manual tunnel starts reject UDIDs or optional route-host strings above 4096
UTF-8 bytes before retaining a gate key, awaiting coordinator state or contacting
a Source. The check also applies to private lifecycle ticket callers. Exact
limits, Unicode byte counts, existing empty/NUL acceptance and routing precedence
remain covered. At most 4096 retained gate keys implies at most 16 MiB of their
logical text; this does not bound allocator capacity, RSS, Source allocations,
arbitrary errors or HTTP request-body allocation. A held coordinator cannot
delay an already-invalid selector. The owned native fixture preserves the old
published Arc, then checks its actual joins, peer EOF, registry/permit drain and
task recovery before asserting the refusal. Private CLI resource decisions now
also reach the manager's typed HTTP error boundary.

With no UDID or IOS_UDID, doctor retains host-only schema 1. An explicit target
selects schema 2 and three independent, fresh existing-trust sessions. Each
probe shares one checked deadline across enumeration, existing pairing-file
load, port connection, lockdown/TLS and its read-only requests:

- `trust_session` requires a completed ProductType GetValue, rather than just
  transport or TLS success.
- `developer_mode` distinguishes an omitted field from explicit true/false.
- `ddi_signatures` observes nonempty Developer/Personalized signatures. It does
  not establish mounted state or image usability, including for personalized DDI.

The source offers no pairing, tunnel activation or setting mutation. Fixtures
exercise the production lockdown and image-mounter wire over duplex/TLS streams,
including a legitimate non-TLS session, service TLS, malformed/missing metadata,
permission/service errors, stalled sends/reads, late results, original errors
acquired while a poll crosses the clock, cancellation and connection EOF.
Report target identifiers are hashed. A safe typed anyhow context keeps the
report's failure category at the final machine-error boundary while preserving
the original IO and lockdown error/downcast. Existing CLI decisions keep their
priority. Neither schema includes private response bodies, credentials or paths.

Additive image-mounter checked methods preserve legacy signatures, wire,
missing-field defaults and enum variants. The checked methods return Option for
omitted metadata and reject wrong field types rather than inventing false values.

Pair-record reads reject nonregular paths before reading and recheck the opened
handle. Unix O_NONBLOCK prevents a FIFO replacement from blocking open. Existing
links to regular files and system-managed readable files remain accepted; no new
token-file ownership policy is applied. Serialized input retains the 1 MiB cap
and Zeroizing wrapper. A real FIFO with no writer is tested in an owned, timed
child; a completion marker prevents a zero-test subprocess from counting as pass.

## Validation

The main agent integrated and formatted proposals from independent worktrees,
then ran Rust checks serially. Agents did not compile. Final affected checks:

- CLI all-feature tests: 1159 passed, zero failed, one existing ignored.
- Image-mounter tests: 35 passed, zero failed.
- Pair-record tests: 15 passed, zero failed.
- `CARGO_INCREMENTAL=0 python3 scripts/validate.py full --python .venv/bin/python`:
  exit zero; Rust 2832 and Python host 13 passed, zero failed, two existing
  ignored tests. CLI 1159, Core unit tests 1508 and public contracts 55 passed.
  Format, all-target/all-feature workspace check, strict Clippy, MSRV 1.80.1,
  66 individual feature checks/four groups, doctests/examples, linked C ABI,
  rebuilt wheel/sdist installation, 73 tooling tests, 118-capability inventory,
  43 raw diagnostic cases, five host workflows, 95 CLI and three backup contracts
  passed. Raw captures include host schema 1, target flag/environment schema 2,
  human output and invalid-selector refusal.

The final full run verified 476 product-file hashes unchanged. Original private
imports, ambiguous fixture dictionary keys and one Clippy finding were corrected
without lowering assertions; failed records remain archived. The earlier full
pass predates the reviewed doctor category fix and FIFO admission and is not the
final acceptance record. During the final run only unused workspace incremental
cache was removed; all builds used CARGO_INCREMENTAL=0, and source, binaries and
evidence were retained.

Logs, commands/exits, source manifests, proposals, the frozen implementation patch
and accepted evidence are in
`target/validation/device-doctor-manual-admission-2026-10-08/`.
Patch SHA256: `d941810fd73366a281d3ff1d4a02b7866c6d810737129c21a74df5db3cfb8cb5`.
CLI SHA256: `a794d39c0acf71ab46ea6034c986958a647b9d1eb1764e97a35be5a6a6e8b27f`.

## Remaining acceptance

Twenty-two real-device checks remain NOT_RUN. RSD services and device routing
remain explicitly not_probed: this invocation has no existing-route input.
Windows USB enumeration, macOS remoted coexistence and platform pairing-directory
diagnostics remain unfinished. Real trust, Developer Mode, DDI usability, service
availability and physical unplug/restart cases need selected devices and states.

Filesystem metadata and regular-file reads can still stall in the OS. Dropping a
probe cannot cancel a running spawn_blocking read, and Tokio runtime exit may wait
for that worker. The FIFO fix does not certify physical cancellation or bounded
CLI exit on a stalled filesystem; Windows special paths and NFS remain gaps.
Other L01/L02 byte, lifecycle, platform and performance acceptance remains active.
No main-task completion or capability maturity upgrade follows. Q04 and all its
children remain excluded.
