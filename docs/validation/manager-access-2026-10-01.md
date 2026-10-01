# S01 manager request admission evidence

Tested implementation `318dab89ed94e6763d0d488e3191bf1bc976ab58`,
2026-10-01, Linux x86_64 / Rust 1.97.1, MSRV 1.80.1, CPython 3.12.13.
Current contract: [management access](../management-access.md).
Prior [host preparation](management-policy-2026-09-30.md) remains historical
evidence for its separate commit. This acceptance covers host request admission,
not an Apple device workflow or other platform's file/ACL behavior.

| Check | Result |
| --- | --- |
| Production manager HTTP | PASS; all endpoints and unknown routes require authentication before extractors; same 401 for missing credentials, fixed/no-store replies, valid list/health/readiness and 403 role/device/port denial |
| Exact device/port policy | PASS; forbidden listing keys filtered; denied upgrades/create/delete do not contact the registered fake upstream; authorized missing device is 404 |
| Reload | PASS; bad private file preserves old credentials; valid replacement rejects old and accepts new token; concurrent/stale/foreign administrators cannot reuse authority |
| Waiting request revocation | PASS; real upstream barrier holds an admitted list request; empty-grant reload cancels it and produces 403, never empty 200 |
| Actual WebSocket bridge | PASS; authenticated production manager relays bytes through a fake upstream to an owned TCP proxy with verified 20-byte IPv6/port preamble; reload and expiry terminate both hops |
| Client credential destination | PASS; actual private-file header accepted, marked sensitive; nonloopback plaintext denied; redirect recipient receives no connection; oversized success body rejected |
| Discovery scope | PASS; list-only manager becomes ready without entering usbmux/discovery; active connect/manage-tunnels scopes explicitly control monitor eligibility |
| Raw output | PASS; 20 CLI/production-service captures, zero original and independently masked token/selector/body/policy hits, correct HTTP/JSON/exit phases, bounded and complete; owned list/server-only live manager skips device discovery |
| Focused checks | PASS; 33 manager tests, six runtime tests plus previous five policy tests in final gate, strict Clippy and 17 tooling failure-path tests |
| Full split gate | PASS; 1612 Rust/eight Python; one existing PBKDF2 ignored; 66 feature configurations/four combinations, seven default/seven full doctests, three snippets, C ABI/19-byte capture, rebuilt wheel/sdist install/import, 118 inventory/actual help, actionlint and diff/staged diff review |
| Real device/platform execution | NOT_RUN; no physical device, macOS/Windows runner or private ACL acceptance supplied |
| Remaining management controls | NOT_RUN; other forward/debug/media entry audit/admission, Host/Origin and browser tickets, legacy GET migration, HTTPS/WSS and per-upstream credentials remain S01–S03 |

```sh
cargo test -p ios-cli --locked cmd::tunnel::tests --no-fail-fast
cargo test -p ios-cli --locked access --no-fail-fast
cargo clippy -p ios-cli --all-targets --locked -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/diagnostics.py --build-cli
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint
git diff --check
git diff --cached --check
```

Full log: `/tmp/ios-s01-manager-final-full.log`. Initial full gate before final
review additions: `/tmp/ios-s01-manager-full.log`, 1610 Rust PASS. The final gate
includes the concurrent-reload and admitted-list-revocation cases. CLI SHA-256:
`d272d4bc81036f6c6096736131f0c75123ba1befa38e14c7a8fbbafe63de5804`.
Capture reports contain only counters/contracts, never subprocess argv, headers,
environment, raw output or generated credentials. Fixtures own their temporary
private paths, loopback listeners and bounded child processes.

The existing userspace proxy accepts its unauthenticated 20-byte preamble on
loopback, and kernel TUN follows OS permissions. Manager grants do not sandbox
local processes accessing those paths. Further forwarding-entry audit is open.
Plaintext manager clients/listeners reject nonloopback; remote use requires a
protected host path. Current upstream calls have no credentials and cannot
connect two policy-mode managers. Browser/native debug admission, protected
federation, platform and device evidence remain tracked in the roadmap. No
push, deployment, device operation or history rewrite occurred.
