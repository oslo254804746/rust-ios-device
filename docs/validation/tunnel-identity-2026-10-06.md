# L02 observed tunnel attachment identities

Tested implementation: `1eb45c25145f5306a5831df6a0c29d494a94ca96`, parent
`dac32cb6111205392051465027572e0eb4519feb`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

The [private source](../../crates/ios-cli/src/cmd/tunnel/source.rs) retains
UDID, native usbmux device ID and USB flag already returned by the existing
MuxClient list operation. Previously it kept only UDID. No new SDK export,
fake SDK constructor, protocol frame or extra daemon query. The four native
connection functions/options, paired discovery and device adapters stay.

The [actual manager](../../crates/ios-cli/src/cmd/tunnel.rs) removes an original
usbmux-route publication when its ID is absent from the observed attached set,
even if the same UDID remains. An original connection with its ID still attached
through another transport and a publication created while the snapshot waited
survive. Route family is stored privately instead of inferred from ID zero;
the synthetic zero fixture does not certify any device-wire validity.

Automatic candidate selection matches existing Core: first USB, otherwise last
non-USB match, keeping each UDID's first discovery position. A candidate carries
the observed expected ID. If the completed device has another ID, its owned
device drops before publication and the scanner observes local OperationCancelled.
Lifecycle cancellation adds no failure backoff; the next observed generation
can retry. Ordinary source IO retains previous reporting/backoff. Existing
serial/watch/permit/deadline/window and HTTP/auth/record contracts stay.

This is a point-in-time snapshot check. Manual starts carry no expected ID.
There is no post-connect attachment query or event listener. A matching old ID
after an unobserved detach, ID reuse/daemon restart or change after the snapshot
remains outside acceptance. Equality is not current physical connection or Apple
ACK proof. Publication freshness/native worker/server joins remain required work.
Retirement/cancellation changes need pre-1.0 release review.

| [Seven actual manager fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/identity_tests.rs) | Evidence |
| --- | --- |
| Same UDID, ID71 becomes72 | Alive old record retires; one replacement call; original owned resource drops |
| Snapshot71, completed72 | No publication; owned result drops; no backoff; subsequent snapshot72 succeeds |
| ID71 still attached alongside preferred USB72 | Original71 remains without replacement |
| First USB | Network71/USB72/USB73 snapshot accepts completed72 |
| Last non-USB | Non-USB71/72 snapshot accepts completed72 |
| Older71 snapshot versus concurrent72 publication | New publication survives; original released once |
| Synthetic zero on usbmux route | Absent attachment removes based on route family; no wire proof |

Private fake snapshot/completed IDs are controlled independently. These exercise
actual TunnelAgentState and owned fake devices. All previous23 assertion bodies
and prior parallel fixtures remain byte-identical. Original HTTP/admission/
federation/bridge/native record grammar, lifecycle coordinator,core/public/access/
dependencies stay. Four files add254/remove18 lines.

Control95029 exited101 with4PASS/2FAIL: alive old ID survived replug and stale
candidate admitted another generation. Temporary unused metadata warnings were
not strict acceptance. Post-change95472 exited101 with E0282 position inference;
no tests ran. After authoritative exit, sole HashMap<String,usize> annotation
fixed it.38985 exited0 with29PASS; zero fixture added before final freeze.
Historical logs/statuses stay; no historical source hash is invented.

Frozen26331 exited0 with all14 gates and frozen-source assertions. Source/result
audit exited0; observation timeout was never counted as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |30/30 PASS |
| Tunnel regression |71 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2539 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI934/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI and Python3.12
wheel/sdist build/install/import pass. Tooling67/help118/raw38/host5/CLI95/backup3
pass.22device NOT_RUN; Chromium/installed3.9/3.13 not rerun. Capability bodies/
maturity and prior evidence stay.

Patch SHA256: `8a1a082a7fc834f0e63c469af60d4d5089aacef826944205376c1e2da45cc467`.
Actual debug ios SHA256: `ad99790c2598d507bc3457c43c75f14415b770468d946ad3280230c8c21c393a`.
Commit matches frozen source. Durable logs,commands,native statuses,hashes/audits
are ignored under `target/validation/tunnel-identity-2026-10-06/`; reference
HEAD/status unchanged.

Continue publication freshness/manual in-flight identity,discovery/whole-scan
totals,metadata/native worker/server joins/resource/platform/device and L04
measurement. Logical snapshot data/owned Drop does not bound RSS or every Core
task. L01/L02/L03–L04/Q04/classes5–9 continue. All31 tasks/subtasks/history and
9 complete/10 not started/12 in progress or conditions remain. No subagent,push,
publication,history rewrite,reference edit or device/Apple service operation.
This intermediate profile does not end the goal.
