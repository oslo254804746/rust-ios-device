# S01 host policy preparation evidence

Tested implementation `b7837cbdeade2b8812b0d85ffb2aad2104cedf33`,
2026-09-30, Linux x86_64 / Rust 1.97.1, MSRV 1.80.1, CPython 3.12.13.
[Management access configuration](../management-access.md) describes the current
host-only commands; server admission remains unfinished.

| Check | Result |
| --- | --- |
| Policy/token unit behavior | PASS, five cases; private Debug, explicit device/port scopes, permission isolation, zero port, expiry equality, wrong token, empty grants, bad schema/ambiguous scopes and partial publication |
| Actual CLI preparation | PASS; generated private credentials, allowed precise check, denied port and credential-bearing help; no default UDID and unavailable usbmux |
| Original output versus masked copy | PASS, 15 total CLI cases; generated token/selector variants checked in RAM, zero hits independently, correct exit/JSON/error contract, no timeout/overflow/incomplete capture |
| Full split gate | PASS, 1599 Rust/eight Python; one existing PBKDF2 ignored |
| Features/docs/artifacts | PASS, 66 feature configurations/four test combinations, seven default/seven full doctests, three snippets, C ABI/19-byte raw capture and wheel/sdist rebuild/install/import |
| Tools/inventory/lint | PASS, 17 tools, 118 capabilities/actual help, strict Clippy, MSRV, actionlint and diff/staged diff review |
| HTTP/WS admission/reload and physical devices | NOT_RUN for this new policy; not wired into server/client routes yet |
| macOS/Windows private-file/ACL acceptance | NOT_RUN; Linux does not supply those platform results |

```sh
cargo test -p ios-cli --bin ios --locked access::tests
cargo clippy -p ios-cli --all-targets --locked -- -D warnings
cargo build -p ios-cli --locked
python3 scripts/diagnostics.py
python3 scripts/validate.py tooling-tests
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint .github/workflows/ci.yml
git diff --check
git diff --cached --check
```

The interpreter/tool paths identify this execution; portable setup is in
[build instructions](../build.md). Full log `/tmp/ios-s01-auth-policy-full.log`;
focused policy log `/tmp/ios-s01-auth-policy-final-tests.log`, captured count-only
report `/tmp/ios-s01-auth-policy-final-capture.json`. Actual CLI SHA-256:
`cc9c48a8f806cedd66e34436cd53a64a57fab823775f10c50b65112cdda8ee5a`.
The full gate includes the final five policy tests, not just the earlier four.

CLI direct rand/subtle dependencies reuse already locked versions; Cargo.lock
changes only those dependency entries. No dependency version was upgraded.
Credential fixture files stay in a unique owned temporary directory and are
removed there; no production credentials/device/policy is selected. Raw output
stays bounded RAM, never a report or archive. Separate files are not a transaction:
the partial-creation test preserves an existing token and leaves the newly
published policy for inspection while returning failure.

This is configuration/host command evidence. It does not authenticate the
current manager, protect a WebSocket, prove live revocation or grant physical
device workflow evidence. S01 and the second roadmap category remain open.
