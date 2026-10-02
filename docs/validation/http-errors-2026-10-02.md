# A02 protected HTTP error headers — 2026-10-02

Implementation `62d1c4ff41d94b9ec849e6cef3972b8d120bfb20`, fourteen related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13. See the
[error contract](../error-contract.md), [compatibility](../public-contract.md)
and [roadmap](../engineering-roadmap-2026-09-30.md).

Protected manager/CDP/WebDriver router errors add fixed version/class/phase/
effects headers while retaining existing statuses, JSON bodies and safe messages.
Native errors are classified before conversion; named local admission, issuer
and manager decisions supply explicit categories. Unknown sources remain Unknown;
effects remain unknown. Successful/informational responses gain no fields.
Approved browser origins can read the four headers; Host/Origin/credential
admission, preflight, HEAD body handling and sanitization retain their boundaries.

| Check | Actual result |
| --- | --- |
| `cargo test -p ios-cli --all-features --locked http_diagnostics -- --nocapture` | Two PASS, including forged headers, raw-secret and original body/status/success-negative checks; `/tmp/ios-a02-http-error-headers.log`, terminal exit 0 |
| Native WebInspector and manager HTTP category scope | Two PASS across owned IO/cancellation/EINTR/opaque/axum/JSON and real Elapsed sources; `/tmp/ios-a02-http-error-native.log`, terminal exit 0 |
| Access and WebInspector scopes | 28 and 39 PASS respectively, zero FAIL/ignored; `/tmp/ios-a02-http-error-access.log` and `-webinspector.log`, terminal exit 0 |
| Final ticket scope | Seven PASS, including three response shapes and future unknown-status negative; `/tmp/ios-a02-http-error-tickets-final.log`, terminal exit 0 |
| `cargo test -p ios-cli --all-features --locked cmd::tunnel::tests -- --nocapture` | Final 41 PASS, zero FAIL, one existing separate-browser ignored; `/tmp/ios-a02-http-error-manager-final-accepted.log`, terminal exit 0 |
| `cargo clippy -p ios-cli --all-targets --all-features --locked -- -D warnings` | PASS; `/tmp/ios-a02-http-error-clippy-final.log`, terminal exit 0 |
| Explicit actual Chromium gate below | One PASS, reload/expiry each 16 checks; `/tmp/ios-a02-http-error-browser-final.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-a02-http-error-full.log`, terminal exit 0 |
| Diff / changed Markdown local links | PASS; 83 links before implementation commit, including the pending roadmap |

The actual protected Rust CDP/WebDriver routers retain injected native permission
and closed-startup-stream errors, with safe original body shapes and source calls
only after admission. Actual manager HTTP/WebSocket requests exercise hop limits,
input rejection, missing registry/tunnel targets and registry capacity without
device work or network dial. The dormant unsupported-transport guard is only
source-reviewed: every currently constructible enum value bypasses that branch.
No device-side unsupported response or entropy exhaustion was fabricated.

The first final-manager compile failed due to a missing existing JsonRequest trait
import. After that correction, one capacity assertion failed because plaintext
DNS was rejected by S03 before reaching capacity. The final fixture uses a valid
HTTPS synthetic registration, performs no dial, and retains the security policy.
Those logs (`/tmp/ios-a02-http-error-manager-final.log` and `-final-fixed.log`)
remain failure history. An earlier browser syntax check failed on a duplicate
local name, corrected before actual browser acceptance.

The explicit browser command uses already available dependencies outside the
checkout; the paths identify this host, not portable requirements:

```sh
IOS_BROWSER_NODE=/home/oslo/.local/share/mise/installs/node/24/bin/node \
IOS_BROWSER_EXECUTABLE=/home/oslo/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome \
NODE_PATH=/tmp/rust-ios-cdp-probe/node_modules \
LD_LIBRARY_PATH=/tmp/ios-browser-libs/root/usr/lib/x86_64-linux-gnu \
cargo test -p ios-cli --all-features --locked browser_ticket_probe_uses_production_manager \
  -- --ignored --nocapture
```

Node 24.19.0 / Playwright 1.63.0 / Chromium 151.0.7922.34 actually read all four
headers on an unauthenticated fetch from an approved origin, retaining replay,
revocation, expiry, byte relay and denied-origin checks. This validates browser
access to headers; it is not a Playwright CDP workflow or physical-device test.

Full gate: **1742 Rust / 12 Python PASS, zero FAIL, two existing ignored**.
Format/check/strict Clippy/MSRV, 66 feature builds/four groups, 7+7 doctests/
three downstream snippets, C ABI/raw plus owned failure, Python 3.12 wheel/sdist,
49 tooling tests, 118 help, 38 raw cases, five host/22 real-device NOT_RUN,
15 CLI and three backup contracts pass. Existing PBKDF2 and separate-browser
ignored cases remain; the browser case passed independently above. Accepted
ios 0.1.13 binary SHA-256:
`a9b56fce623641603f0cd814aa37a78d23eed5d9ed0aa1543b0ed5613c450c4a`.

This host HTTP slice is accepted; **A02 remains in progress**. Pre-router
TLS/parser failures, post-upgrade frames, remote/remaining lost sources,
Python/C interpreter/filtering/void/port boundaries, probe ownership and
stderr/stream/partial semantics remain work. Unknown effects certify no rollback,
safe retry or Apple cleanup. No maturity upgrade, real device operation, push,
publication, history rewrite or reference-checkout change occurred.
