# A02 usbmux native sources — 2026-10-02

Implementation `f8d890522798a444dde7dd600e954f4010f459c4`, five related source files.
Branch `feat/device-service-updates-20260927`, starting clean HEAD `adb95d9`.
Linux x86_64, Rust 1.97.1, MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Four native flattening adapters now use existing MuxError::Io or std::io::Error
with the shared private fixed context. Request serialization retains plist::Error
as InvalidInput; received plist XML/schema faults retain plist::Error as
InvalidData/protocol. TCP connector failure retains the original IO object,
kind and source OS code; deadline expiry retains the actual Tokio Elapsed
under TimedOut. Cached native classifications and the 32-source bound remain.
ConnectionRefused remains generic io; UnexpectedEof is not guessed to mean
device disconnect. CoreError::Mux preserves the nested typed chain.

The three-variant MuxError, root MuxClient, method signatures and grouped feature
definitions stay. Affected Protocol failures return Io; request encoding changes
from protocol to invalid_input, while received codec errors remain protocol.
The TCP context now uses fixed phase text instead of interpolating the endpoint
and timeout duration. IO kind/source remain the programmable observations;
Display prefixes, safe correlations and messages can change. Native IO/codec
Display/source inspection can expose private payloads; context and outer Debug
omit payloads. No new public transport/error export, dependency, CLI schema,
binding, retry, timeout or lifecycle policy is added.

Production comparison is identical after normalizing the four error adapters,
the unused private address parameter and trailing whitespace. Wire length/version/
type/tag, plist request fields, 16 MiB receive bound, short/oversized header guards,
length overflow/u32 sender guards, flush, tags, existing 10-second TCP timeout,
Unix connection policy and successful client flow stay. Created guards keep
Protocol outcomes; integer/tag/admission work remains separate.

| Check | Result |
| --- | --- |
| Core no-default / all-feature `mux::` | 15 / 15 PASS; `/tmp/ios-a02-mux-core-{basic,full}.log`, terminal 0 |
| External public_contract, all / no-default | 20 / 11 PASS; `/tmp/ios-a02-mux-public{,-minimal}.log`, terminal 0 |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-mux-clippy.log`, terminal 0 |
| Production comparison | PASS; `/tmp/ios-a02-mux-code-audit.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-mux-full.log`, terminal 0 |

Five new internal fixtures and one external constructor type check supplement
two existing TCP fixtures updated to inspect native causes. Actual serializer
failure performs no writes; received malformed XML/missing schema consume the
complete owned frame. Closed duplex writes and truncated header/body reads
preserve native kinds. A bound, owned, non-listening TCP socket produces a real
refusal and the original OS code. A pending connector produces real Elapsed
with a five-millisecond test deadline; it does not inject a ten-second production
network black hole. The existing production timeout stays ten seconds.

An owned TCP peer exercises public root read_buid, verifies existing ReadBUID
request fields, returns malformed plist or a valid synthetic BUID, and shuts
down/joins before host drop. The fixture constructs the client privately from
the owned endpoint without modifying process-wide environment; it does not run
the root connect constructor's platform-default endpoint resolution. Its complete
fault/success loop is bounded at five seconds. The external root constructor is
type-checked without polling. These checks do not establish a physical daemon/
device workflow, every native kind, remote cleanup, stream reuse or safe retry.

Final pre-fix regression: **9 PASS / 6 FAIL**, terminal 101,
`/tmp/ios-a02-mux-before-final.log`. The earlier `/tmp/ios-a02-mux-before.log`
also ended 101, but its BUID peer asserted BundleID instead of the existing
serialized BundleId. That fixture assertion was corrected before the final
pre-fix run; production wire keys were preserved. Original logs remain.
The first comparison helper also needed trailing-whitespace normalization at
the newly added test-module boundary; this required no production edit.

Full gate: **1889 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug binary
SHA256 `d8d7bba5fba587cb929bfcfa07e022212398d1b24086d93ce88b8768a8b4686a`.
Source stayed frozen through full and unchanged afterward. No storage failure
or cleanup; about 26 GiB free before full and 24 GiB afterward.

Documentation closeout: capability catalog 118 records, 98 local Markdown links
across five changed Markdown files (baseline `adb95d9`), and whitespace PASS.
Logs: `/tmp/ios-a02-mux-docs-catalog.log` and `/tmp/ios-a02-mux-docs-links.log`.
Source and the accepted full gate remain unchanged.

A02/class 3/overall goal remain active; class 4 has not started. Confirmed next
native loss is internal NSKeyedArchiver plist decoding and its backup metadata
error adapter. DTX/instruments' existing raw/optional decode fallbacks are separate
behavior contracts. Integer conversions/typed guards/created outcomes, other
stream/JSON/callback/binding and L01/L03/A03/platform/device conditions remain.
All 31 tasks/subtasks and maturity remain. No real credentials, devices or Apple
endpoints, push/publication, history rewrite or reference checkout changes.
