# R01 backup boundaries and output acceptance — 2026-10-01

Implementation: `a3336572bca72e1cfd083afde075df6182f36c76`, 17 related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python binding runtime 3.12.13. No physical device selected.
[Architecture](../architecture.md), [output/migration](../backup-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

| Command/check | Actual result |
| --- | --- |
| Baseline `cargo test -p ios-core --all-features --locked backup2` | 65 unit PASS, one filtered integration PASS, one ignored; this filter does not run the whole integration file |
| Final same unit filter | 68 unit PASS, one filtered integration PASS, one ignored |
| `cargo test -p ios-core --all-features --locked --test backup2 --test public_contract` | 11 unchanged protocol interactions and six external consumer cases PASS |
| Default `--no-default-features` unit filter / explicit integration/consumer | 40 unit + one filtered integration PASS; all 11 interactions/five consumer cases PASS |
| `cargo test -p ios-cli --locked cmd::backup::` | 21 PASS: 18 retained, two output-write failures and one blocking-worker/error-source case |
| Core/CLI all-target/all-feature strict Clippy | PASS |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-r01-full.log` |
| Post-review `tooling-tests` / `backup-contracts` scopes | 38 tool tests and three actual CLI cases PASS; owned usbmux trap observes zero dials |
| New backup tool failures under Python 3.9.25 / 3.13.15 | Four PASS each; these are tool tests, not new installed-wheel runtime results |
| actionlint, changed links, diff/staged whitespace | PASS |

Full gate: **1666 Rust PASS, eight Python PASS, zero FAIL, two ignored**.
The existing PBKDF2/browser ignored cases remain ignored; no browser gate was
rerun by R01. Check, strict Clippy, MSRV, 66 feature builds/four groups,
default/full doctests 7+7, three downstream snippets, linked C ABI/19-byte raw
capture, Python 3.12 wheel/sdist rebuild/install, 38 tool tests, 118 actual-help
inventory records, 38 original diagnostics, five regression host cases/22
real-device NOT_RUN and four actual CLI list contracts pass. Backup adds three
actual local CLI contracts, independently of the five workflow-runner host rows.
CLI stays 0.1.13; accepted binary SHA-256:
`8a4fd63fca0e1cc7e1f9ff617a1ec338c55bacff28ef4d69e78fd45dabd7d60f`.

Wire helpers consume supplied streams/values; host helpers consume owned layouts
and return completed path changes. Session state owns transfer accounting,
placeholder tracking/status order and best-effort disconnect. Filtering and
manifest code share the host path policy. All public fields/functions/client
paths and feature availability remain compatible. Nine request structures retain
their field/serialization shapes; all 42 former facade unit-test names were
retained with their owners. Source review found 141 of 150 extracted function
bodies unchanged after whitespace normalization; the nine changed bodies are
host-worker/session boundaries, original join-error sources and exact-file
placeholder relocation/copy. This assists review, not an AST equivalence proof.

The new real backup-protocol flow initially failed with `NotADirectory`: joining
an empty suffix to a moved/copied file appended a trailing separator. The fix
keeps the exact destination; the flow now verifies upload, move/copy, missing
sources, directory listing, remove, disconnect and cleanup without deleting
unrelated data. Initial fixture flag/metadata/UID mistakes and a test dictionary
type-inference failure were corrected before acceptance; they never counted as
passing evidence. Human/JSON/closed-output captures contain 49/199/70 bytes,
zero original secret hits, complete process/fixture cleanup and no panic.

Windows/macOS filesystem fixtures and physical backup/restoration are not
certified by Linux. Started blocking jobs can finish after future cancellation;
staged-file drop cleanup may be synchronous. No stronger device cancellation,
deadline, rollback or cleanup acknowledgement is claimed: those remain in L01
and Q02/Q03/platform acceptance. Output failure can follow completed side effects;
its 101→1 migration and earlier local restore preflight are documented.
R01 host boundary/regression acceptance is complete; R02/R04/A02 and the overall
goal continue. No capability maturity upgrade, pairing, DDI/TSS/restore/erase,
push, publication, history rewrite or reference-checkout mutation occurred.
