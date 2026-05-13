# Build and verification

## Workspace builds

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --exclude ios-py
cargo build --release --workspace --exclude ios-py
```

`ios-py` is a Python extension module and may need a configured Python interpreter. Its
PyO3 extension-module build is exercised by `uvx maturin`; the CI excludes it from normal Rust
build/test jobs and builds wheels separately.

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
