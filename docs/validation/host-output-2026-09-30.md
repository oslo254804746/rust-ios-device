# Host output acceptance, 2026-09-30

Tested implementation: `9e568032ecd34421ebe02ff810e9ed035e09a311`.
Scope: Q03 host writers and output/path contract. Linux x86_64, unprivileged
UID 1000; Rust 1.97.1, MSRV 1.80.1, repository CPython 3.12.13.
No physical-device operation, remote CI run, push or publication occurred.

| Check | Command (all Cargo checks use `--locked`) | Result |
| --- | --- | --- |
| CLI file/image writer | `cargo test -p ios-cli --locked cmd::file::tests` | 30 PASS |
| Backup protocol/path/manifest | `cargo test -p ios-core --all-features --locked backup2` | 64 PASS; existing high-cost PBKDF2 ignored |
| OS trace | `cargo test -p ios-core --all-features --locked ostrace` | 27 PASS |
| Shared move/exclusive directory publication | `cargo test -p ios-core --all-features --locked fs_replace` | 3 PASS on Linux |
| Private staging and real host write failure | `cargo test -p ios-core --all-features --locked staged_file` | 3 PASS; dedicated child gets kernel EFBIG, preserves old bytes and removes its temporary |
| JUnit | `cargo test -p ios-core --all-features --locked --test testmanager junit` | 3 PASS |
| Full gate | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; log `/tmp/ios-q03-final-full.log` |

The full gate includes fmt/check/strict Clippy, 1564 Rust tests (one existing
PBKDF2 ignored), six separate Python host tests, MSRV, 66 feature configurations
and four feature-test combinations, default/full doctests (seven each), three
default-feature downstream snippets, C header/link/ABI calls, wheel install and
sdist rebuild/install, ten validation-tool tests, and actual CLI help/catalog
coverage. An earlier strict-lint run caught a leftover unused preview constant;
the final gate is the result after its removal and the additional write-failure
regression. The earlier gate `/tmp/ios-q03-full.log` predates that last regression
and is not substituted for the final gate.

Observed behavior: staged file/image and backup output preserves previous bytes
on transfer failure/cancellation; remote per-file errors retain DeviceLink's
continuation flow while discarding partial output. Successful file/JUnit/trace
fixtures check size and SHA-256. Unicode and full-length components work; NUL
does not write a path prefix. A no-force race does not overwrite a new target.
Unix read-only-file replacement follows parent-directory rename permissions
and produces private mode 0600. Existing trace destinations, including empty
directories created concurrently, are not overwritten. Cleanup touches only
owned temporary entries and leaves synthetic unrelated files intact.

macOS and Windows execution: NOT_RUN. Their existing system-alias, Unicode,
long/verbatim, Win32 flag/NUL and readonly replacement regressions remain in CI
with new staging/cleanup cases; checked-in branches are not execution evidence.
True UNC: NOT_RUN, no authorized share/fixture was selected. Real AFC,
CoreDevice fileservice, image, backup and trace workflows: NOT_RUN. Host protocol
fixtures are not device observations. The capability catalog retains its
experimental maturity and separate device evidence requirements.

See the [output contract](../output-contract.md) for trusted-parent limitations,
per-file publication, supported-filesystem requirements, cancellation after
commit starts, and post-publication cleanup failure. Q03 awaits the remaining
platform conditions; the second deliverable category remains in progress.
