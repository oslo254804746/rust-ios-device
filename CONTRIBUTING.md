# Contributing

Thanks for considering a contribution. This project is still experimental, so small, well-scoped changes with tests are easiest to review.

## Development setup

```sh
cargo build --workspace --exclude ios-py
cargo test --workspace --exclude ios-py
```

For Python binding work:

```sh
cd crates/ios-py
python -m pip install maturin
maturin develop
```

## Before opening a PR

Run the relevant checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

If a check requires hardware, a specific OS, Developer Mode, a Developer Disk Image, or elevated privileges, describe what you could and could not run.

## Contribution guidelines

- Keep changes focused. Avoid broad refactors mixed with behavior changes.
- Do not commit pair records, certificates, private keys, device backups, UDIDs from personal devices, logs with personal data, or local machine paths.
- Add or update tests for protocol parsing, command-line argument parsing, and service behavior when practical.
- For real-device changes, include the host OS, iOS version range, connection type, and whether the device was trusted, supervised, or in Developer Mode.
- Treat private Apple services as unstable. Document version assumptions and failure modes.

## Code style

The repository uses `rustfmt` with `rustfmt.toml`. Prefer existing crate patterns and error types. Keep public APIs conservative until they have tests and real-device coverage.
