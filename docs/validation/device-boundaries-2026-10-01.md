# R03 device module acceptance — 2026-10-01

Implementation: `f9b96f927431d1e099e121c7347d4a2df7dd5c69`, 17 related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python binding runtime 3.12.13. No physical device selected.
[Architecture](../architecture.md), [roadmap](../engineering-roadmap-2026-09-30.md).

| Command/check | Actual result |
| --- | --- |
| Before refactor: `cargo test -p ios-core --all-features --locked device::` | 57 PASS: 47 device, ten existing coredevice tests also matched by substring |
| Same command after refactor | 58 PASS: all 47 existing device tests retained, one new TCP fixture, same ten coredevice tests |
| `cargo test -p ios-core --no-default-features --locked device::` | 17 PASS |
| Same command with `--features tunnel` | 37 PASS without mdns, userspace or kernel features |
| `cargo test -p ios-core --no-default-features --locked --test public_contract` | Four PASS, expanded root/device path signatures |
| Same consumer with `--all-features` | Five PASS, including mdns paths and aliases |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-r03-modules-full.log` |
| Diff/staged whitespace, changed document links, absence of device `include!` | PASS |

Full gate: **1659 Rust PASS, eight Python PASS, zero FAIL, two ignored**.
The ignored tests remain the preexisting PBKDF2 case and explicit browser gate;
this change did not rerun that browser gate. Check, strict full Clippy, MSRV,
66 feature builds/four feature groups, default/full doctests 7+7, three downstream
snippets, C ABI/19-byte original capture, Python 3.12 wheel/sdist rebuild/install,
34 tooling tests, 118 inventory/help records, 38 original diagnostic cases,
five regression host cases/22 NOT_RUN and four actual CLI list contracts pass.
Earlier A01 installed-wheel results on Python 3.9/3.13 and S02/S03 browser/TLS
results retain their original commits; they are not new R03 runtime results.
CLI remains 0.1.13; built binary SHA-256:
`53e4262f881accee53e2b9d893d5108a59d2cbd554242eb758c4655954068953`.

The facade retains public types and explicit reexports. Connection entry points
select strategies; paired discovery owns trust matching and bounded probes;
lockdown owns credentials, port dialing and its proxy wrapper. RSD probing has
no lockdown proxy ownership. Service routing receives an RSD directory/name and
returns the exact selected name/port/features, or checks in a supplied stream.
Tunnel activation still receives an authenticated byte stream, connection
context and mode. Its constants and nested pairing constants now live with their
owners. Modules use explicit imports and narrow visibility; tests were relocated
to avoid widening helper visibility for a former shared test scope.

Source review preserved all 174 existing extracted function bodies after
whitespace normalization and the same-value discovery timeout reference rename.
The added bodies are an existing temporary-directory helper copied into a second
test owner and the new bounded TCP fixture. This is review assistance rather
than a Rust AST equivalence proof. Runtime regressions preserve canonical/shim
descriptor selection, feature metadata, check-in framing/errors/timeouts, USB
preference, direct/RemotePairing control messages, userspace destination headers
and stable RSD bootstrap. The TCP fixture uses only an owned loopback listener
and verifies bidirectional bytes for both pair-record flag values.

Public connection futures are dropped before polling: that proves source paths
and signatures, not teardown after device I/O. Direct IPv6/kernel endpoint
selection does not certify privileged kernel TUN creation. Synthetic pairing
messages and check-in do not certify an Apple session. Dead-code warnings in
minimal transport-only builds remain in feature logs; full strict Clippy passes.
No public type fields, error variants, ABI exports, CLI output schema, protocol
messages, deadlines or retry policy changed. Historical `device::tests` filters
are no longer broad tests; current regression commands use `device::` as above.

R03's ordinary-module/host-regression/minimal-feature acceptance is complete.
Platform/kernel/device lifecycle evidence remains in B03/Q02/S01/L01, and no
capability maturity was upgraded. No pairing, DDI/TSS/restore/erase, publishing,
push, history rewrite or reference-checkout mutation occurred. R01/R02/R04/A02
and the overall goal continue.
