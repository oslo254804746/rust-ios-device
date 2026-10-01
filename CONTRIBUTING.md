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
uvx maturin develop
```

## Before opening a PR

Run the relevant checks:

```sh
python scripts/validate.py full --python /path/to/python
```

If a check requires hardware, a specific OS, Developer Mode, a Developer Disk Image, or elevated privileges, describe what you could and could not run.

Use `python scripts/validate.py quick --package ios-core` during development.
The full entry point requires stable Rust with rustfmt/Clippy, Rust 1.80, a C
compiler, Python development libraries and uv. See [build.md](docs/build.md)
for all scopes and interpreter selection. Rust workspace tests exclude the
Python packaging feature; embedded Python host tests and wheel imports are
separate required checks. CI uses the same entry point.

## Contribution guidelines

- Keep changes focused. Avoid broad refactors mixed with behavior changes.
- Review the [public compatibility rules](docs/public-contract.md) for Rust
  fields/enum variants, CLI/JSON, C ABI and Python signatures. Preserve the
  representative downstream fixtures or document an explicit migration.
- Do not commit pair records, certificates, private keys, device backups, UDIDs from personal devices, logs with personal data, or local machine paths.
- Add or update tests for protocol parsing, command-line argument parsing, and service behavior when practical.
- For real-device changes, include the host OS, iOS version range, connection type, and whether the device was trusted, supervised, or in Developer Mode.
- Treat private Apple services as unstable. Document version assumptions and failure modes.

## Code style

The repository uses `rustfmt` with `rustfmt.toml`. Prefer existing crate patterns and error types. Keep public APIs conservative until they have tests and real-device coverage.
