# Regression evidence and cleanup

The [capability catalog](capabilities.md) describes implementation/maturity.
[regression-matrix.json](regression-matrix.json) selects representative risks and
their independent preconditions. A host result cannot satisfy a real-device row.
Without an explicit target the runner executes five host cases and writes 22
real-device NOT_RUN records. `--device UDID` selects one target;
`--device auto` accepts only an unambiguous one-device JSON enumeration. Current
enumeration observes transport, not trust, service availability or iOS version.
No device state, version or service is assumed by default.

```sh
cargo build -p ios-cli --locked
python scripts/regression.py
python scripts/regression.py --binary target/release/ios --output-parent /safe/existing/directory
python scripts/validate.py regression
```

`--output-parent` must exist outside the checkout. Default is the system temporary
directory. Every run creates two new private directories: evidence contains only
`report.json`; identity material and the argument correlation key live in the
separate private directory. stdout contains only status counts and the generated
evidence directory basename; combine that basename with the chosen parent to
find the report. Raw output, command/environment values, private paths and
exception text never enter terminal transcripts or artifacts. Files use exclusive
creation (0600 on Unix); directories are 0700 on Unix. Windows ACLs and process
trees remain unverified, so the owned-agent row is NOT_RUN and the gate fails
there instead of silently claiming a complete host workflow.

Reports contain UTC date, host platform/architecture and Python version, code
HEAD, dirty flag, selected binary SHA-256, matrix/runner/capture SHA-256, case
level, transport, scope, preconditions, duration, exit/expected code, raw and
independently masked-copy secret hit counts, contract result, timeout/overflow/
incomplete flags, owned PID/group and cleanup result. A dirty flag means HEAD
alone does not identify the tested working tree; script hashes identify the
actual harness and the binary hash identifies the executable. It is not a clean
commit certification. Archive only audited metadata in an owner-controlled
evidence location; do not add generated reports, credentials or device artifacts
to Git. The runner preserves the report for the user to review.

Arguments go directly through `subprocess` with no shell. Public command/flag
tokens and `/`/`.` are visible; other values have only their index, UTF-8 byte/
UTF-16 unit length and a per-run keyed correlation tag. The key is deleted during
cleanup and is absent from the evidence. This is useful for identifying whether
parameters changed within a run, not for reconstructing a secret path. The argv
fixture compares the child's own `sys.argv` against `/`, `.`, a missing synthetic
path, Unicode and a private synthetic path in RAM. Other cases explicitly say
`direct-subprocess-input`; this does not certify the CLI's internal parsed argv
or Windows Git Bash/MSYS behavior. Follow [AFC/MSYS troubleshooting](troubleshooting.md)
and perform an actual child-argument observation on that platform before closing
FUP-01's platform boundary.

Every ordinary command and agent start/stop uses the same bounded capture core:
256 KiB combined stdout/stderr in RAM, explicit deadline, positive output and
case-specific contract, exact expected exit code, no raw archive. A leak fails
even when the masked copy has zero hits. Timeout, overflow, incomplete capture or
unconfirmed process cleanup cannot pass. CLI invalid-input cases must fail at
the expected parser stage, before device access; exit code alone is insufficient.
Tests inject synthetic secrets into stdout, stderr, contract exceptions and agent
SIGTERM cleanup, and exercise timeout/overflow and failed artifact audit paths.
This detects registered secret representations; it is not a proof that arbitrary
unregistered device content is safe. New adapters must register all relevant
synthetic/identity material and use boolean/allowlisted metadata contracts.

Linux agents are trusted, non-detaching owned fixtures in a new session. Capture
checks and terminates the owned group even if the leader exited and descendants
closed their pipes. `/proc` observation distinguishes terminated zombies from
running members. No process-name kill is used. A process deliberately creating
another session is outside this group guarantee and is not a supported agent
adapter. Unix systems without that observation fail conservatively when group
absence cannot be established; Windows has only the existing CLI-leader capture
boundary, not a certified general process-tree supervisor.

File cleanup uses filesystem APIs, not an uncaptured shell: only registered
regular files under the unchanged absolute private root are unlinked. Symlinks,
changed root/file identities, directories and unknown files are preserved and make cleanup
fail. Evidence has an exact allowlist; an unexpected file prevents report
publication and remains untouched. No recursive deletion, existing pairing
material or user file cleanup occurs. These checks share the repository's
trusted parent-directory boundary and are not an atomic defense against an
adversary concurrently swapping directory components.

The trusted Python `Session.run`/`agent` API remains reusable. The CLI accepts no
arbitrary commands. `regression_adapters.py` constructs real product argv with
the root `--udid`, loads a hash-bound fixture manifest, and runs finite operation
sequences only when their separate operation scopes are granted. Missing service, permission, identity, platform or fixture
must remain NOT_RUN/INCONCLUSIVE with a reason. Adding a matrix row or observing
source code does not constitute execution. Use 60 seconds for ordinary device
commands, 90 seconds for special connection cases, at most two retries after the
initial attempt and at most 15 minutes of investigation; stop earlier when a
repeated failure adds no evidence. Never retry around a permission failure or
repeat a state-changing operation implicitly.

FUP-04 keeps each Windows output subcase independent, including ordinary long
paths, explicit verbatim paths and separately authorized real UNC. It requires
an owner-placed small ordinary-AFC synthetic file, known size/SHA-256 and exact
read scope. NUL cannot cross CLI argv and retains host-only coverage. Display/
HID, clipboard, application, XCTest and Cryptex workflows require their own
explicit scopes; this runner does not infer those from general roadmap consent.
The current [FUP review](real-device-followup-tasks-2026-08-30.md) records what
already exists and what is missing.

## Repair boundary, 2026-10-10

`--fixture-dir` loads `manifest.json` (at most 64 KiB), with exactly `version: 1`,
`target_udid`, `fixtures`, `observations` and `evidence`. Its selected target must
match the CLI target. Observations/evidence never grant the `--scope` permissions.
Evidence entries contain a root-relative regular-file path and SHA-256; symlinks,
paths outside the fixture root, mismatched package hashes and mismatched IPA
`CFBundleIdentifier` values are rejected. The evidence report also records the
adapter source hash and admitted manifest hash.

Supported fixtures are `afc-fixture` (`remote_path`, `size`, `sha256`),
`test-application`/`test-runner` (`path`, `sha256`, `bundle_id`), `test-bundle`
(`runner_bundle_id`, `xctest_config`, nonempty exact `tests`, `timeout_secs`),
`authorized-unc-share` (`directory`) and `existing-rsd-route` (`address`, optional
`proxy_port`). Package and evidence paths stay inside the selected root. The UNC
fixture names an already existing, explicitly selected share directory; the
runner creates one exclusive child there and removes only that child's known
files. Read-only observations still require `read-scope`; app/XCTest operations
require their own matrix scopes. An AFC fixture is an owner-placed remote file,
so this runner never creates or removes remote AFC content.

The file adapters perform real `file pull` operations, compare full size/SHA-256,
check existing-file refusal and Windows native read-only-file refusal, preserve
directory markers, inspect exact staging residue, exercise force/recovery and
Unicode/long/verbatim output paths, then remove only exact locally owned files.
Unknown residue is preserved and fails cleanup. Application sequences inspect
preexistence, install, query the real bundle ID, launch, kill and uninstall only
an absent bundle owned by this run; cleanup also runs after partial installation
failure. XCTest uses finite `runxctest --wait` selectors and checks the JUnit
case selection and log completion footer.

Private MAC/altIRK correlation, exact Cryptex ownership, Display/HID state
restoration, rich clipboard watch/deduplication, same-client InstallCoordination
wire recovery, and queued/direct/legacy/passive RSD mode selection still require
production observers. They remain specifically NOT_RUN; no `info`, directory
listing or unrelated successful query substitutes for those workflows. macOS
remoted needs an explicitly selected existing RSD endpoint. All physical-device
flows, actual authorized UNC and platform-specific processes remain unverified
until separately executed. Q02 is therefore still incomplete.

`test_regression_adapters.py` can check an already built CLI with
`IOS_REGRESSION_CLI`. It never builds Rust. Its Windows protocol fixture runs the
actual executable through synthetic usbmux, Lockdown TLS and AFC peers using a
private `ALLUSERSPROFILE`, checks all original worker joins and strict EOF, and
labels its results `localhost-protocol-fixture`. Those results cannot set
`real_device_executed` or satisfy a physical-device matrix row.
