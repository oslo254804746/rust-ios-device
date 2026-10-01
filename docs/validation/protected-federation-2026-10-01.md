# Protected federation acceptance, 2026-10-01

Tested implementation: `39fd900b0785ee0baa893c6f97afacbba90f9e74`.
Linux x86_64; Rust 1.97.1/MSRV 1.80.1; Python 3.12.13.
Device/iOS/physical transport/state: **NOT_RUN**. Test TLS material is public,
test-only and unrelated to pairing; never trust its CA/key outside owned fixtures.

| Check | Result and scope |
| --- | --- |
| Private upstream configuration | Strict version/fields/normalization/duplicates/bounds, relative private token, safe Debug, scope-bound headers: PASS |
| Actual HTTPS/WSS | Verified private CA accepts HTTP and binary WS. Untrusted/expired/wrong-name certs fail before backend HTTP; additional trust on A does not authorize B: PASS |
| Redirect/credential isolation | Actual HTTP and WS redirects refuse without contacting destination. Credentials never attach to a different authority or transport: PASS |
| Two production managers | Distinct incoming/middle/leaf credentials, TLS listing/WS, selected root/hop/binary bytes and middle reload→EOF: PASS. Third leaf/TCP are owned fixtures |
| Wrong credentials | Production aggregate/selection explicitly 502; no empty-success fallback. Unregistered selector still 400: PASS |
| Manager regression | 39 PASS, one explicit browser ignored; cycles/hop/close/EOF/backpressure and native/ticket scopes preserved |
| Upstream security | Six focused tests PASS, including real HTTP/WS/TLS and certificate trust isolation |
| Browser regression | Chromium 151.0.7922.34/Playwright-core 1.63.0, explicit test PASS, reload and expiry variants ×16 checks; browser→production manager host relay |
| Full gate | 1651 Rust/eight Python PASS, zero failure, two ignored (existing costly PBKDF2; explicit browser gate separately PASS) |
| Full tooling/artifacts | 66 feature builds/four groups, 7+7 doctests/three snippets, C ABI/19-byte capture, wheel/sdist rebuild/install/import, MSRV/strict Clippy, 17 tooling and 118 catalog/help entries PASS |
| Original diagnostic capture | 38 actual CLI/service cases, original and independently masked copies have zero hits. New private-upstream startup denial has no listener/discovery/secret path output |

Commands:

```sh
cargo test -p ios-cli --locked cmd::tunnel::tests
cargo test -p ios-cli --locked access::upstream
cargo clippy -p ios-cli --all-targets --locked -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
IOS_BROWSER_NODE=/home/oslo/.local/share/mise/installs/node/24/bin/node \
IOS_BROWSER_EXECUTABLE=/home/oslo/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome \
NODE_PATH=/tmp/rust-ios-cdp-probe/node_modules \
LD_LIBRARY_PATH=/tmp/ios-browser-libs/root/usr/lib/x86_64-linux-gnu \
  cargo test -p ios-cli --locked browser_ticket_probe_uses_production_manager \
    -- --ignored --nocapture
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint
git diff --check
git diff --cached --check
```

Sanitized test logs: `/tmp/ios-s03-upstream-tests.log`,
`/tmp/ios-s03-protected-federation-tests.log`, `/tmp/ios-s03-federation-regression.log`,
`/tmp/ios-s03-browser-regression.log`, `/tmp/ios-s03-federation-full.log`.
Tested CLI SHA-256: `fc718d5b92f2a25f21ed1a5485edc5cf90d3d49a575b112bbd192df360ea27f9`.

Lock changes enable tokio-tungstenite 0.24.0 TLS and add only webpki-roots 0.26.11
compatibility wrapper referencing existing 1.0.6; no existing package version
upgraded. Full MSRV remains PASS. TLS fixtures were generated offline, then
embedded under cfg(test), so running these tests needs no OpenSSL CLI or device.
Valid fixtures expire in 2046; the separate expired fixture is intentionally past
its validity. An eventual fixture refresh must retain these negative tests.

Aggregate `/` now fails if any attempted upstream fails; local `/tunnels` remains
local-only. HTTP/WS follow no redirects or environment proxies. Plaintext DNS/
remote roots, including old `http://localhost`, need literal-loopback migration
or HTTPS. Native listeners still require a separately deployed protected host
forward; this gate does not establish/certify SSH host keys or a real reverse
proxy deployment. CA/token changes require restart; incoming reload only revokes
incoming policy leases. These boundaries are normative in
[protected federation](../protected-federation.md).

Other platform file/ACL/TLS execution, deployed transport/certificate evidence,
physical-device topology/EOF, F03 CDP page and F05 browser workflows remain
pending. No real pairing/DDI/TSS/destructive device action, push, publication,
history rewrite or reference-checkout modification occurred.
