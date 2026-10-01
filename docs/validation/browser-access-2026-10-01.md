# Browser access acceptance, 2026-10-01

Tested implementation: `826ebf3312e54fb52651969ad5d45fbee914aa5b`.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python 3.12.13,
Node 24, Playwright-core 1.63.0 and Chromium 151.0.7922.34 (cached revision 1234).
This is S02 host access/byte-relay evidence. Device/iOS/state are **NOT_RUN**.

| Check | Result and boundary |
| --- | --- |
| Shared access | 21 PASS: prior policy/boundary plus ticket scopes, replay/concurrency, Origin/complete target, malformed/ambiguous inputs, 16 KiB body and capacity, expiry/reload and CORS |
| Production manager | 36 PASS; one explicit browser gate ignored in ordinary test invocation. Ticket/native bridges retain byte/EOF/backpressure/hop/revocation behavior |
| Production debug routers | 28 PASS. Ticket page upgrade/revocation with page lock held before all device connection; no Apple page assertions |
| Actual Chromium | One explicit test PASS, reload and expiry variants with 16 checks each; native browser fetch/WebSocket, fresh context, approved UI origin |
| Browser rejection | Unauthenticated bootstrap, forbidden device/port/body Origin, missing ticket, long-term URL credential, tamper/replay and disallowed UI Origin refuse admission |
| Browser relay | Exact binary bytes through Rust production manager, owned upstream and TCP fixture; existing 20-byte project preamble/EOF checked; no device |
| Browser revocation | Original grant expiry and successful policy replacement close the active stream and invalidate pending tickets |
| Raw captures | Existing 37 actual CLI/service cases PASS, extended with live CDP issuance and denied ticket/long-term URL HTTP; manager admin cannot issue connect ticket. Raw and independently masked captures each have zero hits |
| Browser child output | Bounded original stdout/stderr checked for token/device and any 64-hex ticket; no raw archives. Fixed version/count/result reports only |
| Full gate | 1642 Rust/eight Python PASS, zero failure; two ignored in automatic Rust gate: existing high-cost PBKDF2 and explicit browser test (separately PASS) |
| Other full checks | 66 feature builds/four combinations, default/full doctests (7+7), three snippets, strict Clippy/MSRV, linked C ABI/19-byte error capture, wheel/sdist rebuild/install/import, 17 tooling, 118 capability/help entries and actionlint PASS |

Commands:

```sh
cargo test -p ios-cli --locked access::
cargo test -p ios-cli --locked cmd::tunnel::tests
cargo test -p ios-cli --locked cmd::webinspector
cargo clippy -p ios-cli --all-targets --locked -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
node --check scripts/browser-ticket-probe.cjs
IOS_BROWSER_NODE=/home/oslo/.local/share/mise/installs/node/24/bin/node \
IOS_BROWSER_EXECUTABLE=/home/oslo/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome \
NODE_PATH=/tmp/rust-ios-cdp-probe/node_modules \
LD_LIBRARY_PATH=/tmp/ios-browser-libs/root/usr/lib/x86_64-linux-gnu \
  cargo test -p ios-cli --locked browser_ticket_probe_uses_production_manager \
    -- --ignored --nocapture
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
python3 scripts/validate.py diagnostics
/tmp/ios-validation-tools/actionlint
git diff --check
git diff --cached --check
```

Browser dependencies are prerequisites outside the checkout, not packaged
dependencies. The cached browser initially lacked shared libraries; Ubuntu
packages were downloaded/extracted under `/tmp/ios-browser-libs` and used through
LD_LIBRARY_PATH. No system installation/reference edit was needed. Default
Playwright browser revision was absent, so the gate uses the explicit tested
executable. On another host install matching Playwright/browser prerequisites,
then point these path-only variables at them. Missing prerequisites fail the
explicit gate; ordinary full validation's ignore is not a browser PASS.

Logs contain sanitized test reports: `/tmp/ios-s02-browser-tests.log`,
`/tmp/ios-s02-ticket-manager-tests.log`, `/tmp/ios-s02-ticket-debug-tests.log`,
`/tmp/ios-s02-access-tests.log`, `/tmp/ios-s02-tickets-full.log` and count-only
`/tmp/ios-s02-tickets-diagnostics.log`. Tested CLI SHA-256:
`5263e21510a3b134d25e1a890f4e8bd0840754266c06dd2e856a5a851f13c33b`.

The issuer intentionally returns a short-lived credential in API JSON; that is
not an automatic diagnostic. Browser developer tools/access logs may record its
URL, so the controlled UI/forwarder must disable such logging. No console/HAR/
video/raw URL artifacts are collected by this gate. Linux process-group cleanup
is exercised; general Windows descendant supervision/platform browser acceptance
remains pending. Firefox/Safari are not certified by this Chromium run.

Existing manager/CDP/Selenium Linux boundaries are accepted. Future F05 browser
entry points still require integration/acceptance. S03 protected transport and
per-upstream credentials, physical device workflows, orderly Apple cleanup and
F03 Playwright/Puppeteer CDP initialization/page operations remain pending. This
test is not the earlier standalone fake-CDP client probe and does not report any
Apple selectors/workflows as successful. No publication, push, history rewrite,
reference modification or destructive device operation occurred.
