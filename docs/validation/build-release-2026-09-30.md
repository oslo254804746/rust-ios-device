# Build and release validation — 2026-09-30

This is host evidence for B01–B04. The implementation commit and task statuses
are recorded in [the roadmap](../engineering-roadmap-2026-09-30.md#14-持久化实施与交付记录).
No device was opened, paired or modified; no package or release was uploaded.

Environment: Linux x86_64, kernel 7.0.0-34-generic; rustc 1.97.1;
Cargo MSRV 1.80.1; GCC 15.2.0; maturin 1.15.0; CPython 3.9.25,
3.12.13 and 3.13.15. The interpreter paths below are placeholders for the
selected runtime, not committed host configuration.

| Command / check | Result |
| --- | --- |
| `python scripts/validate.py full --python <python3.12>` | PASS; all required steps completed |
| Locked all-target/all-feature workspace check and Clippy | PASS; no Rust lint warnings in full-feature configuration |
| Split Rust workspace tests | PASS: 1546, FAIL: 0, ignored: 1 (existing high-cost PBKDF2) |
| Python host tests (full entry point, 3.12) | PASS: 6 |
| `python scripts/validate.py python-host --python <python3.9>` | PASS: 6 |
| `python scripts/validate.py python-host --python <python3.13>` | PASS: 6 |
| Automatic minimal feature matrix | PASS: 66 configurations, including default, each service, group, alias, transport and implicit `lz4_flex`; four representative test configurations |
| Default/full core doctests | PASS: 7 each, including five `compile_fail` encapsulation checks |
| Downstream README Rust examples | PASS: 3; no core service features enabled |
| Python README and binding-doc snippets | PASS: syntax compilation; real-device execution NOT_RUN |
| Linked C smoke and listing example compilation | PASS; NULL, UTF-8, enum/struct layout, status and NULL-free contracts |
| `python scripts/artifacts.py native --target x86_64-unknown-linux-gnu` | PASS; CLI version, shared/static libraries, C header, C call against release library, archives, checksums and manifest |
| `python scripts/artifacts.py verify-native --directory dist` | PASS |
| Wheel metadata/installation plus sdist build/installation (full, 3.12) | PASS; independent venvs and installed public API/error checks |
| Release abi3 wheel import on 3.9 and 3.13 | PASS on Linux; `cp39-abi3-manylinux_2_38_x86_64` |
| `python scripts/validate.py tooling-tests` | PASS: 5; failure exit, missing tool, checksum corruption, missing required library, every publisher's transitive mandatory gates |
| `actionlint .github/workflows/ci.yml` (1.7.7) | PASS, including shellcheck |
| `cargo package -p ios-core --list --allow-dirty --locked` and CLI equivalent | PASS |
| `cargo publish -p ios-core --registry crates-io --dry-run --allow-dirty --locked` | PASS; upload aborted by dry-run |
| CLI registry dry-run / multi-package verification | FAIL; resolves published core 0.1.15, missing local APIs; release condition below |
| `git diff --check` | PASS |
| macOS, Windows and ARM Linux candidate matrices | NOT_RUN locally; configured in CI, no remote run claimed |

The first documentation extraction run failed because the validator removed
`#` from Rust attributes; the extraction rule was corrected and both focused
and full runs passed. The first native check found missing CLI `--version`;
the command was added and the native release was rebuilt and passed. A subsequent
format check caught the new attribute's wrapping; rustfmt and full validation
then passed. These failures were not ignored or reclassified as successes.

Minimal builds retain warnings from shared tunnel MTU types, pairing/H2/TLV
helpers and feature-dependent imports in `device`/`fetchsymbols`. Logs are
written to `target/validation/features/*.log` and uploaded by CI even on failure.
The host Cargo configuration also emits a pre-existing deprecated filename
warning; it was not modified. Full-feature strict Clippy passed.

All three publication branches wait for tag validation and native/Python
candidate jobs. Those jobs wait for check, feature-check, MSRV, native tests
and Python smoke. Failed or skipped prerequisites block publishers; the only
`always()` is a log-upload step. Candidate jobs run on PRs without publication.

Remaining release conditions: run the configured platform matrices; choose
a new release version consistently for core/CLI/Python/C and the workspace
dependency; make the matching core available in the registry and reverify CLI
packaging. The current already-published 0.1.13 cannot be replaced, and the
registry-resolved 0.1.15 does not contain this checkout's new APIs. No local
workspace success or package file list establishes that registry check.

Local raw command output was captured under `/tmp/ios-b01-b04-acceptance.log`,
`/tmp/ios-b04-native-final.log` and the task-specific `/tmp/ios-b04-*` logs.
Those are temporary host logs, not durable remote evidence. This checked-in
summary and reproducible entry points preserve the outcomes; generated
archives and libraries remain ignored build artifacts.
