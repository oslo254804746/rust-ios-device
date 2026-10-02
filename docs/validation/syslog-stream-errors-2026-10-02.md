# A02 syslog stream failures and partial records — 2026-10-02

Implementation `7b6394ec63dd68c79d290a1f470618ecf34110c3`, five related files.
Independent test-fixture correction `1de103eb79d66dad6905fdcb1e737d76a8f00c2d`.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[usage](../usage.md), [roadmap](../engineering-roadmap-2026-09-30.md).

Syslog's private stream driver now returns native relay read errors through the
normal CLI failure path rather than printing a diagnostic and returning success.
EOF retains UnexpectedEof/Io; it is not inferred to prove device disconnect.
Earlier stdout records remain, and unterminated relay messages are not emitted.
Final failures use safe human text or the opt-in JSON stderr envelope, phase
syslog, exit 1 and unknown effects. Existing count/filter and chosen capture-time
stops remain successful; the capture window still starts after service setup.

Default stdout remains compact NDJSON with existing fields, one newline per
matching record. Embedded newlines/quotes are escaped inside each JSON object.
Raw and parsed human bytes are preserved, including the existing parsed extra
newline for a message that already ends with one. Record writes now check flush
as well as write; failures can leave a partial last record. The existing fixed
stderr progress line also uses checked write/flush. It remains ordinary stderr
text alongside the opt-in final error object. The linked migration tells scripts
that formerly treated a relay failure as exit 0 to inspect status and preserve
valid prior records, without assuming rollback/retry or completed cleanup.

| Check | Result |
| --- | --- |
| `cargo test -p ios-cli --all-features --locked cmd::syslog::` | Final 16 PASS, zero FAIL/ignored; `/tmp/ios-a02-syslog-host-fixed.log`, terminal exit 0 |
| `cargo test -p ios-cli --all-features --locked cmd::webinspector::bridge_tests::` | Corrected independent fixture: four PASS; `/tmp/ios-a02-syslog-bridge-isolated-pages-fixed.log`, terminal exit 0 |
| `cargo test -p ios-cli --all-features --locked` | Final 645 PASS, zero FAIL, one existing browser ignored; `/tmp/ios-a02-syslog-cli-final.log`, terminal exit 0 |
| CLI strict all-target/all-feature Clippy | Final PASS; `/tmp/ios-a02-syslog-clippy-final.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | Corrected full PASS; `/tmp/ios-a02-syslog-full-fixed.log`, terminal exit 0 |
| Review | Driver/rendering are private; public args, fields and wire keys are unchanged. 103 changed local Markdown links and whitespace PASS; this is not a universal ABI proof |

Six new owned host cases drive the same CLI helper through the real core relay
decoder. Four native read failures arrive after two complete NDJSON records;
tests inspect retained native IO and class, safe actual envelope rendering, exit
field 1 and unknown effects. EOF after a valid record and an unterminated suffix
emits only that valid record, then fails with Io. Filter/count stops succeed and
drop the reader before its terminal error. An actual idle owned duplex expires
after the selected one-second capture window, emits nothing and observes peer
EOF. Partial output and flush failure retain BrokenPipe, stop after the first
emission and leave the observed prefix/full first record. Human raw/parsed bytes
match their prior formats. Reader ownership drops exactly once in tested paths.

These are driver/decoder/output/envelope fixtures, with dispatch wiring reviewed.
They do not run an actual paired-TLS syslog CLI process, physical relay, OS signal,
service-startup stderr-failure fixture or Apple cleanup workflow. Shared checked
writer regressions pass in full; the owned duplex EOF establishes local resource
drop only. Core message-limit boundary, other streams, application mutation JSON,
bindings and stronger L01/L03 lifecycle behavior remain separate pending work.

Failure history is retained. The first human fixture expected one newline where
the existing parsed format adds a newline to an already newline-ended message;
correcting the expected bytes kept product compatibility (15 PASS/one FAIL in
`/tmp/ios-a02-syslog-host.log`). The first full failed at the CLI Rust stage:
644 PASS/one page-bridge FAIL/one ignored; remaining gates did not execute.
`/tmp/ios-a02-syslog-full.log` is failure history, not accepted full evidence.

The failed bridge passed alone and in five original paired reruns. Source review
found the independent page/browser fixtures share a process-wide page-lock target
and a 100 ms budget; contention is an inference, not a fully reproduced cause.
The browser fixture now has a distinct page ID, preserving original deadlines,
listing/setup/target, Runtime response, redaction and teardown assertions. Product
CDP locks are unchanged. An initial Drop-struct update expression failed E0509
(`/tmp/ios-a02-syslog-bridge-isolated-pages.log`); explicit construction corrected
it. Final bridge/CLI/Clippy/full gates pass with the isolated fixture.

Full gate: **1769 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
CLI change. The ios 0.1.13 SHA-256 is
`f62516c5f070eb050dfb6cc9a5e95a8347e3b777678d4ef525ddb9fce684a3ed`.

This host slice is accepted; A02/third class and the 31-task goal remain active.
No capability maturity upgrade, device operation, push, publication, history
rewrite or reference-checkout modification occurred. Next is the registered core
syslog terminator/size boundary, then remaining JSON/stream/binding/source work.
