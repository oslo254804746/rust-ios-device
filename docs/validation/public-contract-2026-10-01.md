# A01 public compatibility acceptance — 2026-10-01

Protocol correction: `8d07bc977a8c8510085e783a3f6b7cb39ec83c5b` (one file).
Contract/fixture implementation: `076e9097afe763456f0b063edd96d1b4bc9e2397`
(18 related files). Branch `feat/device-service-updates-20260927`.
[Policy](../public-contract.md), [roadmap](../engineering-roadmap-2026-09-30.md).

Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python binding runtime 3.12.13;
additional installed-wheel smoke on 3.9.25 and 3.13.15. No device selected.

| Check | Result |
| --- | --- |
| `cargo test -p ios-core --locked --test public_contract` | Four default-feature external consumer tests PASS |
| `cargo test -p ios-core --locked --no-default-features --features icon_service,screen_capture --test public_contract` | Five tests PASS; aliases expose identical public types |
| `cargo test -p ios-core --locked mux::protocol::tests` | Four PASS, including canonical/legacy ProductID and attach/missing cases |
| `cargo clippy -p ios-core --all-targets --all-features --locked -- -D warnings` | PASS |
| `python3 scripts/validate.py ffi` | C signatures/status/layout/ownership and original diagnostic capture PASS |
| `python3 scripts/validate.py python-package --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | Wheel/sdist build/install/import and signature/resource smoke PASS |
| Candidate wheel installed through `artifacts.py python --directory /tmp/ios-a01-wheel-candidate-8ardyet1 --python <3.9 or 3.13>` | Both actual runtimes PASS; no device calls |
| `validate.py tooling-tests` / `compatibility` | 34 tools PASS; four actual CLI fake-usbmux contracts PASS |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; log `/tmp/ios-a01-contract-full.log` |
| Clean committed CLI compatibility rerun | Four PASS at `076e909`; worktree clean before doc closure |

The full gate records **1658 Rust PASS, eight Python PASS, zero FAIL, two
ignored**. The ignored tests are the preexisting expensive PBKDF2 case and the
explicit browser probe; the prior S02/S03 browser evidence is retained, and this
change did not rerun it. Default/full doctests 7+7, three downstream snippets,
66 feature builds/four feature groups, MSRV, full check/Clippy, linked C ABI,
wheel/sdist rebuild/import, 118 inventory/help records, 38 raw diagnostic cases,
Q02 five host cases/22 NOT_RUN and four new CLI compatibility cases all pass.
Workflow actionlint, diff/staged whitespace and local document links pass.
CLI version stays 0.1.13; binary SHA-256:
`f488aab365eb475ce915cef85d1ea9b35358269908482e68a2e0edd9043cd5c1`.

The external Rust consumer locks existing public paths/fields and exhaustive
matching while performing only metadata/codec/channel operations. Device futures
are dropped without polling; this is source compatibility, not a cancellation
or physical tunnel teardown test. Its all-feature suite has five passing tests.
C smoke assigns all 14 functions to typed pointers and links/runs existing null,
UTF-8, output reset and matching-free paths; it does not certify every architecture.
Installed Python checks names/order/defaults, exposed patch/resource methods and
the documented factory-only construction boundary. An initial check incorrectly
included internal `restore_all`; inspection/actual wheel proved it is not a
public method, so the contract/test now preserve actual `install`/`restore`.
The corrected wheel and sdist checks pass.

Actual CLI processes contact only a newly owned loopback fake usbmux endpoint,
which accepts bounded ListDevices frames and supplies two synthetic USB/Network
entries or an empty list. JSON field/type and human-mode fixtures are checked;
stderr must be empty, exit zero and original synthetic environment secret hits
zero. Explicit selector result data is inspected in RAM and not archived or
printed by the tool. The fixture reproduced the earlier canonical `ProductID`
being ignored as PascalCase `ProductId`, then passed after the isolated decoder
fix. Missing metadata still yields zero and the old alias still works.
Go reference type `ios/listdevices.go` was read only; reference commits are
recorded outside the checkout at `/tmp/ios-a01-reference-provenance.json`.
The separate reference plist did not contain that key and is not asserted to
provide a live wire observation. No reference source or device data was copied.

Candidate compatibility profiles do not upgrade Q01 device maturity. A02 stable
error fields, L01 deadline/cancellation, A03 SDK resources, platform ABI/file
acceptance and D02/1.0 device workflows remain their own required work. The
policy forbids silent retrofit of private fields/non_exhaustive to existing
constructible types and documents versioned replacements/migrations. No public
type visibility, error variants, ABI exports, CLI result schema or package version
was altered. No physical device, pairing, DDI/TSS/restore, push, release, history
rewrite or reference-checkout mutation occurred. A01's rules/representative host
scope is accepted; class three and the overall goal continue.
