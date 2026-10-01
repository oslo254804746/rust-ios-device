# S01 host doctor manager credential acceptance

Implementation `2c8c8c818669517f53161229b0840066bbdcd024`, 2026-10-01,
Linux x86_64 / Rust 1.97.1, MSRV 1.80.1, CPython 3.12.13.
Contract: [host diagnostics](../troubleshooting.md) and
[management access](../management-access.md).

| Check | Result |
| --- | --- |
| Doctor private credential | PASS; actual file/header accepted by owned HTTP fixture; wrong token, 403 and missing file are failed checks; absent token retains optional-manager warning |
| Diagnostic privacy | PASS; report contains counts and fixed/safe errors, no device/address/token or remote error body |
| Existing probe boundaries | PASS; redirect/oversized body/timeout tests retained; proxy bypass, bounded response and per-probe deadlines preserved |
| Actual CLI with production manager | PASS; owned list/server-only policy skips device discovery; doctor legal token reports manager ok, missing file reports failed; intentionally invalid usbmux makes overall exit 1 in both and remains independently checked |
| Raw capture | PASS; 26 actual CLI/service cases, original and masked variants independently zero, correct JSON/exit/phase, no timeout/overflow/incomplete capture |
| Focused/full gates | PASS; six doctor tests, strict Clippy; 1617 Rust/eight Python, one existing PBKDF2 ignored; 66 feature builds/four combinations, seven default/seven full doctests, three snippets, C ABI/19-byte capture, wheel/sdist rebuild/install/import, 17 tooling cases, 118 capabilities/actual help, MSRV, actionlint and diff/staged diff review |
| Device doctor/readiness, other platforms | NOT_RUN; this remains host scope, without device trust/Developer Mode/DDI/service acceptance or macOS/Windows ACL execution |

```sh
cargo test -p ios-cli --locked cmd::doctor::tests
cargo clippy -p ios-cli --all-targets --locked -- -D warnings
python3 scripts/diagnostics.py --build-cli
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint
git diff --check
git diff --cached --check
```

Full log `/tmp/ios-s01-doctor-full.log`, count-only raw capture report
`/tmp/ios-s01-doctor-diagnostics.log`; actual CLI SHA-256:
`219211b96c019f1cd0f71adb7153cfe4fbac92dfbc17b6b1122561f5e2b34066`.
Fixtures own private temporary paths and bounded loopback processes. Raw output
stays in bounded RAM and never enters reports or archives. Other probes retain
their own status; a manager PASS is not device readiness or a global doctor PASS.
CDP/Selenium/browser admission, protected per-upstream federation, platform and
physical-device evidence remain unfinished under the roadmap. No device
operation, push, publication or reference/history change occurred.
