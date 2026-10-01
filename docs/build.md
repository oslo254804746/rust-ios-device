# Build and verification

## Workspace builds

```sh
python scripts/validate.py full --python /path/to/python
# During development, test only the affected package:
python scripts/validate.py quick --package ios-core
cargo build --release --workspace --exclude ios-py
```

The core workspace test intentionally excludes `ios-py`. Test the binding
separately with Python 3.9+ development headers and a runtime selected by
`PYO3_PYTHON` when necessary. Its `extension-module` feature is packaging-only:
`cargo test --workspace --all-features` must therefore use
`--exclude ios-py`, while `uvx maturin` enables the feature automatically for
wheel builds.

The validator runs without Bash on Linux, macOS and Windows (Python 3.9+).
`--python` selects the CPython runtime for PyO3 compilation and embedded host
tests; it defaults to `PYO3_PYTHON`, then the running interpreter. The validator
derives `PYTHONHOME` and the platform library search path from that runtime,
without writing local configuration. The runtime must provide development
libraries. Install Rust 1.80 alongside stable, rustfmt, Clippy, `uv`, and a C
compiler (`cc` or a configured MSVC developer prompt). `CC` can select the C
compiler executable. A missing prerequisite or failed check stops with a
nonzero exit status and names the failed check.

The full set is format, locked all-target/all-feature check and Clippy, split
Rust/Python host tests, MSRV, every Cargo-reported core feature individually,
representative feature-group tests, default/full doctests, downstream README
examples, linked C ABI smoke, wheel/sdist metadata plus wheel installation,
validation-tool tests, capability inventory, raw host diagnostic capture,
[host workflow evidence/cleanup](regression.md),
[public CLI compatibility fixtures](public-contract.md), and
[local backup output/failure fixtures](backup-contract.md).
Feature warnings and test output are retained in `target/validation/features/`;
feature cuts can expose unused imports/dead code from shared transports. These
are recorded warnings; full-feature Clippy still rejects warnings.

Each step also has a focused scope: `fmt`, `check`, `lint`, `rust-tests`,
`python-host`, `msrv`, `features`, `docs`, `ffi`, `python-package`, `tooling-tests`,
`capabilities`, `diagnostics`, `regression`, `compatibility`, `backup-contracts`. CI calls
these same scopes. Linux results do not establish macOS/Windows support;
CI is configured for native/C checks on three systems and Python host/wheel checks on 3.9
and 3.13. No test in this entry point pairs, installs to or modifies a device.
The `regression` and `backup-contracts` scopes currently require Linux for owned process-group
checks; other platforms retain those checks as NOT_RUN and return nonzero. The Linux
CI check job runs it. This condition does not disable the separate native/C and
Python platform jobs. A standalone run needs an already-built `target/debug/ios`;
the full gate builds it through `capabilities` and `diagnostics` first.

## Candidate release artifacts

Build and verify the current host's target before any publication:

```sh
python scripts/artifacts.py native --target x86_64-unknown-linux-gnu
python scripts/artifacts.py verify-native --directory dist
python scripts/validate.py python-package --python /path/to/python
```

Choose the matching native target on macOS/Windows/ARM Linux. CLI version,
required shared/static libraries (plus the Windows import library), header,
archive members, SHA-256 and per-target manifest must agree. Missing artifacts
fail the check. Python checks verify the `cp39-abi3` floor, distribution name,
version and source contents, install into a temporary venv and exercise the
public API/error contract without a device. CI additionally builds/installs
the sdist with `artifacts.py python --directory dist --build-sdist`; the
`python-package` scope runs that source-build check locally too.

Candidate native archives, Python wheels and sdist are built and checked on
PRs. All three publishing jobs wait for every candidate matrix, whose prerequisites
include format/check/Clippy, Rust tests/docs/C ABI, MSRV, every minimal feature,
and Python host/wheel smoke. Tags must match the workspace version. Publishing
steps are separate from validation; local validation never uploads or publishes.

## Windows dependencies

OpenSSL must be installed via vcpkg with static linking:

```powershell
vcpkg install openssl:x64-windows-static-md
```

Set the following environment variables before building:

```powershell
$env:VCPKG_ROOT = $env:VCPKG_INSTALLATION_ROOT
$env:VCPKGRS_TRIPLET = "x64-windows-static-md"
$env:OPENSSL_STATIC = "1"
```

## Linux dependencies

On Debian/Ubuntu-style systems:

```sh
sudo apt-get update
sudo apt-get install -y libssl-dev pkg-config usbmuxd
```

The exact package names may vary by distribution.

## Python binding

```sh
uv pip install rust-ios-device-tunnel
```

For local wheel development from this repository:

```sh
cd crates/ios-py
uvx maturin develop
```

If PyO3 selects the wrong interpreter, set `PYO3_PYTHON` in your shell rather than committing it to `.cargo/config.toml`.

## C FFI

```sh
cargo build --release --package ios-ffi
```

The public header is in `crates/ios-ffi/include/ios_rs.h`.

## Packaging checks

Before a crates.io release, run package checks per publishable crate:

```sh
cargo package -p ios-core --list
cargo package -p ios-core
cargo publish -p ios-core --dry-run
cargo package -p ios-cli --list
cargo package -p ios-cli
cargo publish -p ios-cli --dry-run
```

Run checks in dependency order: `ios-core` first, then `ios-cli`. In a local workspace,
`cargo package -p ios-cli` and `cargo publish -p ios-cli --dry-run` can still fail before
the matching `ios-core` version is available from the crates.io index because publish
verification resolves registry dependencies instead of trusting the sibling path dependency.
Treat that as an expected local limitation; after `ios-core` is published and indexed,
rerun the `ios-cli` package and dry-run checks before publishing it.

The 2026-09-30 checkout still uses the already published version `0.1.13`.
Its current CLI also depends on APIs absent from the registry-resolved
`ios-core 0.1.15`, so the CLI's registry dry-run fails even though the local
workspace and native archive pass. Release preparation must choose a fresh
version for both publishable crates and their workspace dependency, publish
and index the matching core, then repeat CLI verification. An existing version
cannot be overwritten. This registry result is separate from local build
correctness; see the roadmap delivery record for the pending release condition.
