# Companion empty registry and remote symbol catalog regression

Baseline: `ef5fd8cd6cb1c529a001d60bcef8d3e0923a21d9`.
Device: D2, iPhone XR (`iPhone11,8`), iOS 18.7.10 (`22H374`), USB.

`companion list` treated `GetDeviceRegistry`'s `Error=NoPairedWatches` as a
protocol failure. Both the classic service and RSD shim returned this value.
The core client now returns an empty list for this specific response while
retaining other device errors.

`symbols list` already selected the userspace tunnel and
`com.apple.dt.remoteFetchSymbols` for iOS 17+. Its failure occurred after the
RemoteXPC handshake: the device advertised 71 files, whose even transfer streams
exceeded the framer's limit of 32 unknown streams. The client now admits the
bounded catalog's even file streams before consuming entries. The default
unknown-stream ceiling, byte budgets, reset and EOF handling remain active.
Admission stores a compact upper bound and does not open or acknowledge files.

Validation:

- Added regressions failed against the original implementation and passed after
  the changes: unpaired watch response, 71-file catalog, unrelated stream flood.
- 37 companion, 88 H2, 11 fetchsymbols unit and 10 fetchsymbols integration tests
  passed (146 total).
- `cargo build -p ios-cli --all-features --locked` passed.
- `cargo check -p ios-core --locked --no-default-features --features fetchsymbols`
  passed, with unused-code warnings in the reduced feature configuration.
- `cargo fmt --all --check` and `git diff --check` passed.
- Fresh CLI device regression: companion returned `[]` in 3/3 attempts; symbols
  returned 71 unique paths in 3/3 attempts. Every path set matched a fresh
  pymobiledevice3 comparison on the same device.
- Five further read-only checks verified device type, OS version, build, battery
  fields and the RSD symbol-service capability.

Binary SHA256:
`49D1A4DC7BA697EE378C253396EE70FBCF2D89DC2F66489A9265717FDD41B448`.
Exact commands and private evidence are outside the repository in run
`20261028-161151-codex-fixes` under the user's `ios-device-tests` directory.
No restore, reset, reboot, trust, DDI, application or system-network changes were
performed. The original test run was preserved; its matrix was not rerun in full.

The accompanying evidence audit identified at least seven original PASS rows
whose recorded commands did not complete their planned scenarios. Those are
tracked separately from these two product fixes; original matrix totals must
not be presented as complete device acceptance.
