# R04 plist conversion and stdout acceptance — 2026-10-01

Implementation: `c365d5569cb94cc9bade2d17499a37040ae9736a`, 85 related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python binding runtime 3.12.13. No physical device selected.
[Encoding/migration](../plist-json-contract.md), [architecture](../architecture.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

| Command/check | Actual result |
| --- | --- |
| Before extraction: `cargo test --workspace --exclude ios-py --all-features --locked plist_json_` | 17 unchanged historical-shape fixtures PASS: CLI eleven, core four, FFI two |
| After extraction/cursor refinement: same command with filter `plist_json` | 20 PASS; zero selected cases in filtered integration targets are not counted as passing integration evidence |
| CLI MDM unit filter, `/tmp/ios-r04-mdm.log` | Seven existing MDM cases including secret-key behavior PASS |
| Minimal public consumer `--no-default-features --test public_contract` | Six PASS |
| Final core `--all-features plist_json` | Seven PASS, including both borrowed/owned 4096-level traversal |
| Core `--all-features --test accessibility_audit --test public_contract` | Seven service interactions and eight external consumer cases PASS |
| CLI `--all-features result_writer_tests` | Four PASS: partial line, missing newline, prompt flush failure, fixed XML bytes/retained partial-write IO source |
| Pre-final CLI all tests | 618 PASS, one existing browser ignored; final full gate includes the additional XML test |
| Core/CLI/FFI all-target/all-feature strict Clippy | PASS |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-r04-full.log` |
| Actual CLI `scripts/compatibility.py` | Eight PASS; historical four output fixtures plus list JSON/human/help/version closed-pipe cases |
| `scripts/backup_contracts.py` | Three retained local-output contracts PASS; zero device dials and complete owned cleanup |
| Three new tool negative tests, Python 3.9.25 / 3.13.15 | Three PASS each; tool tests do not certify a newly installed wheel on those runtimes |
| Changed local Markdown links, diff/staged whitespace | 129 links and whitespace PASS |

Full gate: **1693 Rust PASS, eight Python PASS, zero FAIL, two ignored**.
Existing PBKDF2 and independently gated browser cases remain ignored. Check,
strict Clippy, MSRV, 66 feature builds/four groups, default/full doctests 7+7,
three downstream snippets, linked C ABI/19-byte raw capture, Python 3.12
wheel/sdist rebuild/install, 41 tool tests, 118 actual-help inventory records,
38 original diagnostic cases, five regression host cases/22 real-device NOT_RUN,
eight actual CLI list/help/version contracts and three backup contracts pass.
CLI remains 0.1.13; accepted binary SHA-256:
`d6e9bc8e8f86fad2578c5e3508bd1b26e39cb77c577ba328904faacdb5e84d73`.

The old R02 binary passed the original four CLI fixtures and failed all four
new closed-output contracts. In particular, help/version formerly discarded
the stdout failure and exited successfully. The R04 binary returns exit 1
with a safe IO diagnostic and no panic. Actual captures have zero original
secret hits, bounded output and complete process cleanup. Tool negative cases
reject silent success, panic text and a missing private executable.

Fourteen private conversion owners and direct serde fixtures freeze complete
Data/Date/Uid/integer/non-finite/nested values. Thirteen delegates select explicit
shared policies; MDM retains its key-aware redaction traversal and shares only
ordinary scalar encoding. Direct plist serde and XPC/NSObject/model contracts
remain distinct. No C symbol/header/layout or existing public signature changed.
The new non-exhaustive DataEncoding and private policy fields are additive.

Conversion uses depth-first cursors and moves owned ordinary strings/keys. It
does not add input parsing, transport, redaction, truncation or hidden limits.
The depth test unwinds fixtures/results iteratively; it does not certify
recursive JSON serialization/drop. A future unknown plist variant is not
constructible in locked plist 1.7.1, and the legacy integer fallback is also
unreachable through its public constructors/parser. Their behavior is recorded
and reviewed, not misreported as executed tests. No measured performance
improvement is claimed.

A lexical comparison of HEAD before R04 and the implementation found all 603
result macro calls in 61 files retain their argument tokens and line/formatted
kind (`/tmp/ios-r04-output-diff-review.json`). This aids review, not an AST
equivalence proof. Twenty-eight private human helpers now return Result and
57 call sites propagate errors. Six XML stdout sites retain serializer bytes
and the extra newline with checked flush; Bluetooth capture checks final flush.
Backup reexports the common writer. Existing operation failures retain their
unsuccessful status when best-effort stderr error reporting also fails.

Initial reverse-iterator compilation and the first stdout migration's 84
helper/import errors failed before correction. These failures were never
accepted as evidence. The final shared cursor, fixed helpers, output fixtures
and full gate all passed before the implementation commit.

R04 host conversion/output acceptance is complete. Regular stderr callbacks,
typed classification, machine error mappings, stream framing and partial-operation
semantics remain A02 work; orderly device/stream cleanup remains L01. An output
failure can follow completed side effects and does not promise rollback/retry.
macOS/Windows output failures, physical service workflows and stronger lifecycle
acceptance are not certified by Linux fixtures. No maturity upgrade, pairing,
DDI/TSS/restore/erase, push, publication, history rewrite or reference mutation
occurred. Class three and the overall goal continue with A02.
