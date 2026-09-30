# Error and discovery formatting acceptance, 2026-09-30

Tested implementation: `c8ed4c9c252d50f493d1117c9089982c384aa3aa`.
Scope: the second S04 implementation slice, not complete S04 acceptance.
Linux x86_64, Rust 1.97.1 / MSRV 1.80.1 / CPython 3.12.13.

`cargo test -p ios-core --all-features --locked debug_`: nine PASS, including
four new formatter checks. `... pair_record_error_diagnostics`: one PASS.
`... device::tests`: 47 PASS after correcting the missing-pair-record
Unsupported error construction to use a diagnostic identifier.
`rustfmt --edition 2021 --check crates/ios-core/src/device/tests.rs`: PASS
(the file is included rather than discovered by Cargo's formatter).

Synthetic values are passed directly to ordinary/pretty Debug and the targeted
Display implementations before checking the resulting bytes. No sanitized copy
is substituted for those formatter results. Fields and explicit selector JSON
remain available for their stated API purpose. These checks cover the changed
formatters; they are not raw-process-output or live HTTP/Python exception
acceptance. Error sources remain available to programmatic callers, and arbitrary
Protocol/IO Display text still needs safe handling at diagnostic boundaries.

Final full gate:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
PASS, log `/tmp/ios-s04-error-debug-full.log`: fmt/check/strict Clippy; 1569 Rust
tests, one existing PBKDF2 ignored; six separate Python host tests; MSRV; 66
feature configurations/four feature-test combinations; default/full doctests
(seven each); three downstream examples; C ABI; wheel install and sdist
rebuild/install; ten validation-tool tests; 118 capability entries versus
actual CLI help. No physical device, remote CI, push or publication occurred.

NOT_RUN: complete raw stdout/stderr secret capture, generic error/source-chain
boundary checks, HTTP/CDP/Selenium and Python synthetic-secret end-to-end
acceptance, timeout/cleanup capture, and device observations. The ordinary
Python/C build/smoke gate cannot stand in for those specific S04 checks.
RemotePairingRecord's remaining directory diagnostics and the complete log-site
audit stay tracked in the roadmap; no full-log safety claim is made here.
