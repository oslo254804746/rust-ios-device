# S02 host HTTP boundary and method acceptance

Implementation `4f868548fba6423e4060c0a1bb681a12d5222822`, 2026-10-01,
Linux x86_64 / Rust 1.97.1, MSRV 1.80.1, CPython 3.12.13.
Contract and standards: [HTTP boundaries](../http-boundary.md).

| Check | Result |
| --- | --- |
| Shared production boundary | PASS; manager/CDP/Selenium check actual listener IP/localhost and bound port before auth/extraction/device operations |
| Actual HTTP/WS | PASS; rebinding host, wrong port, null/unapproved/duplicate Origin, missing/duplicate Host, absolute URI mismatch and proxy spoofing rejected before counted handlers; safe envelopes/no-store/HEAD |
| Configuration and IPv6 | PASS; exact/bounded independent allow-host/origin reject wildcard/userinfo/path/query/fragment/duplicate/invalid port before listen; real IPv6 equivalent expanded origin works at actual port |
| Read-only legacy GET/HEAD | PASS; authenticated 405/Allow POST notice without parsing selectors or shutdown/tunnel creation; positive POST shutdown notifies; before/at/after 2026-11-01 UTC cutoff tested |
| POST start boundary | PASS; existing transport/response types retained; JSON denies ungranted device before discovery and safely rejects invalid enum; no granted device creation performed |
| Explicit debug selector | PASS; missing selector fails without global first-device discovery/usbmux; other WebInspector selection separately retained |
| Raw production CLI/service gate | PASS; 37 cases, including invalid config/missing selectors and three-service Host/Origin refusals, same-origin native headers and GET/POST migration; original/masked variants zero, correct phase/exit, no timeout/overflow/incomplete capture |
| Full gate | PASS; 1634 Rust/eight Python, one existing PBKDF2 ignored; 66 feature builds/four combinations, seven default/seven full doctests, three snippets, C ABI/19-byte capture, wheel/sdist rebuild/install/import, 17 tools, 118 capabilities/actual help, MSRV, actionlint and diff review |
| Browser/device/platform | NOT_RUN; ticket/bootstrap/CORS still unimplemented; no actual browser/Rust-bridge, physical tunnel start/Safari/automation/teardown or macOS/Windows execution |

```sh
cargo test -p ios-cli --all-features --locked access::http_boundary::
cargo test -p ios-cli --all-features --locked cmd::tunnel::tests -- --nocapture
cargo test -p ios-cli --all-features --locked cmd::webinspector:: -- --nocapture
cargo test -p ios-cli --all-features --locked debug_listeners_require_a_selector_without_global_device_discovery
cargo clippy -p ios-cli --all-targets --all-features --locked -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/diagnostics.py --build-cli
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint
git diff --check
git diff --cached --check
```

Focused checks: four boundary, 35 manager, 27 debug and one explicit-selector
tests PASS. Full log `/tmp/ios-s02-http-full.log`; count-only capture
`/tmp/ios-s02-http-diagnostics.log`; tested CLI SHA-256:
`11b7229d9890d251548fc3e2f6b3ded6edda3e1ff29c1a0bd3b167e3aba33e5c`.
Owned fixtures admit no physical device operation. Authorized live debug calls
are metadata/reload only; controlled SIGTERM remains distinct from device ACK.

Review fixed an out-of-range port appearing omitted in Authority::port and the
earlier CLI first-device lookup preceding debug startup validation. Earlier S01
fixtures supplied explicit synthetic selectors; they did not cover that omitted
path. An old GET scope assertion was migrated to the equivalent POST device-scope
check after the intended method change; no authorization coverage was removed.
Federation hop/cycle/EOF/close/backpressure regression remains passing.

S02 remains in progress: short-lived single-use browser handshakes, controlled
credential bootstrap/CORS and actual browser acceptance are next. Native bearer
and allowed Origin do not provide those steps. S03 isolated upstream credentials/
protected transport and platform/device evidence remain pending. No push,
publication, destructive device, history or reference operation occurred.
