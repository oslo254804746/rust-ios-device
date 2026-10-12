# Q02 protocol observer continuation — 2026-10-12

This package follows the application/XCTest CLI gates accepted at
`c98c819153e5fe3f12f4c31845f6026f089a7492`. The new production observers are
awaiting the root task's combined Rust build and native gates. This document
does not count mocked response validation as a native operation or a
physical-device result. No device mutation was performed.

## InstallCoordination

The opt-in command is:

```text
ios --udid SELECTED --error-format json apps install-record BUNDLE
    --observe-zero-budget-recovery --rsd-address [IPv6]:PORT
    [--rsd-proxy-port PORT] --timeout-secs 10
```

It requires an explicit selected identity and already established RSD route.
It bypasses default device discovery, verifies a fresh current RSD directory,
matches the selected UDID, then opens the canonical advertised service through
the shared `ExistingRsdSession`. It does not create, stop or claim ownership of
the external tunnel. Handshake, service bootstrap and queries share an absolute
operation deadline and cumulative service wire limit.

The additive Core `query_with_budget` and `query_observed_with_budget` methods
use local budgets and preserve the instance's default timeout. The original
`InstallRecord` fields and struct-literal contract are unchanged. The observed
result preserves `DBSequence`'s actual `Int64`/`Uint64` tag before converting its
value to `u64`. A negative signed value still fails. The CLI issues a genuine
zero-budget query, requires its typed timeout and unchanged usable client, then
issues the finite query on that same service connection.

The machine result contains the sequence, bounded sizes and SHA-256 fingerprints,
actual wire enum and timing; it does not echo raw DBUUID, URLs or persistent
identifier bytes. Cleanup closes the shared session's original sockets under a
separate reserve, shuts down writes, drains within the owner's limit and
requires actual peer EOF. `opened_sockets == closed_sockets == 1` and
`strict_eof: true` are checked before success. Primary and cleanup errors are
retained separately. Elapsed time alone cannot establish no business I/O.

The matrix fixture `install-record-observation` has exactly `path` and SHA-256
of a root-relative bounded JSON file. The JSON supplies `version: 1`,
`bundle_id`, `db_sequence`, `db_uuid_sha256`, `persistent_identifier_size`,
`persistent_identifier_sha256` and `wire_type: "int64"`. The signed matrix row
cannot be satisfied by an unsigned response. An `existing-rsd-route` fixture
and separate `protocol-observation-scope`/`synthetic-operation` scopes remain
required; observations never grant them.

`regression_rsd_fixture.py` serves actual H2/XPC bytes on two owned IPv6
localhost sockets. It receives the native host RSD handshake, verifies the
service bootstrap, accepts exactly one finite Query, and emits either signed
or unsigned native replies. It serves no observer-shaped JSON. The original
worker/socket EOF and join requirements remain strict. The actual executable
gate covers signed same-client recovery, unsigned tag preservation, wrong
current identity, missing canonical service and negative signed reply. It is
pending the rebuilt executable.

## Display/HID

The opt-in command is:

```text
ios --udid SELECTED --error-format json hid --confirm --timeout 30 observe-stop
    --expected-status-sha256 HEX --confirm-stop-all
```

The matrix requires `display-input-scope` plus separate `stop-all-media-scope`
and a `display-hid-baseline` fixture. That fixture hash-binds a bounded JSON
file with `version: 1`, `status_sha256` and
`scenario: "center-contact-cancel"`. The status fingerprint is SHA-256 of the
encoded XPC value wrapped with message ID zero and ALWAYS_SET|DATA flags. The
whole observed value is compared before/after; a mismatch fails instead of
normalizing unknown or volatile fields into a passing state.

One original `NativeScope` retains the kernel connection, HID service and
media session outside cancellable futures. The observer compares the current
server status with the approved baseline before starting, starts native media,
writes the exact synthetic Down/Cancel contact sequence, stops its original
media tasks and requires an actual CoreDevice stop reply, then obtains a fresh
after-status and closes the original native registry. Partial start/contact
intents are retained before a possible write so failure/cancellation still
attempts bounded cleanup.

Universal HID reports are one-way writes. `hid_reports_flushed` and
`local_contact_closed` are local ownership receipts. The result explicitly says
`hid_device_ack: "unavailable-in-protocol"`; it does not certify remote contact
state. `media_status_restored` refers only to the two actual media-status
observations. Strict stop does not turn EOF/reset/broken pipe into an ACK, and
does not use the client session UUID as a device-issued stream token. The
explicit stopAll authorization is necessary for the current observer path.

Core strict-stop gates exercise actual XPC reply bytes, native EOF refusal and
the original UDP drain task join. The whole kernel/UDP/device observer has not
been executed on this Windows host and remains NOT_RUN. Legacy permissive stop
compatibility is separate from the strict observer's proof.

## Negative receipt validation

The adapter refuses exit-zero results when restoration hashes differ, media
restoration is false, an original task/native owner remains unjoined, the stop
reply is missing, local contact closure is false, cleanup is incomplete or a
device ACK is falsely claimed. Install results additionally require the
original socket cleanup receipt, signed wire tag and both no-business-I/O and
same-client facts. A changed admitted fixture fails before running the CLI.

`test_regression_observer_contracts.py` executed seven pure Python contract
tests PASS. These patch response capture deliberately and are validator tests,
not production protocol evidence. An initial admission-test setup omitted its
service observations; it was corrected to supply them rather than skipping the
real precondition check. Native gates and their raw command logs will be
recorded by the root task after the combined build.

Test entry points (they never build Rust):

```text
python -m unittest discover -s scripts -p test_regression_observer_contracts.py -v
python -m unittest discover -s scripts -p test_regression_install_observer.py -v
```

All physical-device rows, the POSIX clipboard watch on Windows, and the
separately developed private-MAC/altIRK and RSD-mode observers keep their own
validation state. No unexecuted row is relabelled PASS by these scripts.
