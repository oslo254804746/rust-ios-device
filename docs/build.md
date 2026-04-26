# Build and verification

## Workspace builds

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --release --workspace --exclude ios-py
```

`ios-py` is a Python extension module and may need a configured Python interpreter. The CI excludes it from normal Rust build/test jobs and builds wheels separately with maturin.

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
```

Repeat in dependency order. Do not run `cargo publish` until package contents, metadata, and dependency versions are reviewed.
