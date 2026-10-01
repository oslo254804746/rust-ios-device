# S01 raw forwarding entry acceptance

Implementation `e6ddf5c2bbe7a12142dfa4dfc05b6afa1133a925`, 2026-10-01,
Linux x86_64, Rust 1.97.1 / MSRV 1.80.1, CPython 3.12.13.
Contract and entry audit: [management access](../management-access.md).

| Check | Result |
| --- | --- |
| Forward bind preflight | PASS; wildcard IPv4/IPv6, remote IP and DNS name rejected before device resolution |
| WDA proxy preflight | PASS; nonloopback and either zero port rejected before testmanager/WDA startup |
| Dproxy preflight | PASS; nonloopback rejected before service connection/capture creation |
| Raw screenshot stream | PASS; nonloopback rejected before listener or device capture; reports actual bound address |
| Raw CLI capture | PASS; 24 total cases, including four new early refusals with synthetic selectors at maximum verbosity; original/masked hits zero and exit/phase/budget/complete contracts satisfied |
| Full split gate | PASS; 1616 Rust/eight Python, one existing PBKDF2 ignored; strict lint/check/fmt and MSRV, 66 feature builds/four test combinations, seven default/seven full doctests/three snippets, C ABI/19-byte capture, wheel/sdist rebuild/install/import, 17 tooling cases and 118 capabilities/actual help |
| Source audit | Userspace TUN binds fixed `127.0.0.1:0` and accepts a 20-byte IPv6/little-endian port preamble without credentials; kernel routes use host permissions |
| Physical device, other platforms, SSH/TLS path | NOT_RUN; no device selected or remote host configured, Linux does not supply macOS/Windows/ACL evidence |
| Debug/browser/federated admission | Unfinished under S01–S03; no claim that this raw-entry change protects those paths |

```sh
cargo test -p ios-cli --locked nonloopback --no-fail-fast
cargo test -p ios-cli --locked invalid_bind_and_ports_fail_before_wda_device_startup
cargo clippy -p ios-cli --all-targets --locked -- -D warnings
python3 scripts/diagnostics.py --build-cli
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint
git diff --check
git diff --cached --check
```

Full log `/tmp/ios-s01-listener-full.log`; sanitized capture report
`/tmp/ios-s01-listener-diagnostics.log`. Actual CLI SHA-256:
`728de4bcd69da6687922252fa924f2c04d3b9dafbe829b8c8493d3d255e053e1`.
The main gate retains manager admission/expiry/reload and federation transport
fixtures from [prior acceptance](manager-access-2026-10-01.md).

These native listeners keep their existing raw protocols on loopback. A local
process can use a userspace proxy without a manager grant; kernel access follows
routes/firewalls. This is an enforced network boundary and explicit local trust,
not an OS sandbox or proof of device service compatibility. Nonloopback host
options now fail; remote native clients need protected host transport. No device
pairing, WDA startup, capture, erase/restore, push or deployment was performed.
