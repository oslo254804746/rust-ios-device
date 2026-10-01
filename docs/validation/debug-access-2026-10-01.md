# S01 CDP/Selenium host admission acceptance

Implementation `0bb58a208de73b16997d8354920bbe7118c94dc5`, 2026-10-01,
Linux x86_64 / Rust 1.97.1, MSRV 1.80.1, CPython 3.12.13.
Contract: [management access](../management-access.md) and
[CDP compatibility](../cdp-compatibility.md).

| Check | Result |
| --- | --- |
| Startup boundary | PASS; implicit trust, conflicting modes, missing policy and remote plaintext rejected before bind/device access; localhost and IPv6 metadata use actual ephemeral listener |
| Actual production HTTP routers | PASS; every debug route rejects missing/invalid/duplicate credentials before path/body/WS extraction; correct fixed-device debug metadata succeeds, other roles/devices fail 403 |
| Policy replacement | PASS; debug-only cannot reload; bad private policy preserves admission, authorized replacement accepts new credentials and rejects old ones; correct diagnostic envelope and no raw private path |
| Actual production page WS lease | PASS; authenticated upgrade blocked on an owned page lock closes on expiry/reload without reaching a device; no claim of Apple/CDP command compatibility |
| Pending HTTP revocation | PASS; production WebDriver JSON extraction waiting for a partial body returns safe 403 on reload |
| Stored authorization resources | PASS; generic production session store with owned host resources cancels another credential's pending request on original-owner expiry/reload, including escaped session ID; removal precedes cleanup and stalled cleanup times out/releases resource |
| Child ownership | PASS; dropping CDP page owner aborts its task/releases held lock; dropping session store releases resources without watcher ownership cycles |
| Actual CLI raw diagnostics | PASS; four debug startup denials and both production metadata/reload listeners at maximum verbosity; all 32 CLI/service captures have zero original/masked secret hits, correct phase/HTTP/exit and no timeout/overflow/incomplete capture |
| Full gate | PASS; 1627 Rust/eight Python, one existing PBKDF2 ignored; 66 feature builds/four combinations, seven default/seven full doctests, three snippets, C ABI/19-byte capture, wheel/sdist rebuild/install/import, 17 tooling cases, 118 capabilities/actual help, MSRV, actionlint, diff/staged diff review |
| Physical/platform/browser workflows | NOT_RUN; no selected device, actual Safari/Remote Automation sessions, Apple teardown ACK, Playwright/Puppeteer Rust-bridge workflow, browser ticket/Host/Origin or macOS/Windows execution |

```sh
cargo test -p ios-cli --all-features --locked cmd::webinspector:: -- --nocapture
cargo test -p ios-cli --all-features --locked cmd::tunnel::tests -- --nocapture
cargo clippy -p ios-cli --all-targets --all-features --locked -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/diagnostics.py --build-cli
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint
git diff --check
git diff --cached --check
```

Focused WebInspector tests: 27 PASS; existing manager regression: 33 PASS.
Full log `/tmp/ios-s01-debug-full.log`; count-only capture report
`/tmp/ios-s01-debug-diagnostics.log`. Tested CLI SHA-256:
`30f1e22f1a859dd1586bb53bac7a6b4b4ec6f069f632651453a2903d63d3cd8e`.
Fixtures own private files, local ports and bounded processes. Debug captures
end by controlled SIGTERM (Linux exit -15), explicitly expected and distinct
from capture timeout; this supplies neither graceful shutdown nor device ACK.
Only local metadata/reload are authorized in the live captures. Raw bytes stay
in bounded RAM, without archives or request/credential dumps.

Active debug grants cover the whole selected device, not per-user session
ownership. Revocation cancels pending work and removes stored admission; already
performed device operations are not rolled back. Session cleanup has a 100 ms
host budget; CDP cancellation aborts child tasks/streams. Orderly Apple teardown,
session quotas and actual client workflows remain lifecycle/F04 acceptance.
S01 platform/device evidence and S02/S03 remain unfinished; no push, publication,
pairing, DDI, TSS, destructive device or reference/history operation occurred.
