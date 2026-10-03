# A02 discovery service native sources — 2026-10-03

Implementation `89bcd3f81f6e0617e8a1526f3147feb5e7197876`, twelve source files,
starting HEAD `8a08f22` on `feat/device-service-updates-20260927`.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Twenty-two native error adapters now retain their original objects:

| Service | Retained sources |
| --- | --- |
| WebInspector | Client three, Inspector three and Automation five Elapsed; page listing-key and application PID two ParseIntError; frame length one TryFromIntError: fourteen |
| Companion | Send, roundtrip, listener and forwarding stop four Elapsed; signed-to-unsigned and u16 port two TryFromIntError: six |
| NotificationProxy | Wait and next-event two Elapsed |

The PID formatter, Companion listener match arm and negative-port `.ok()` were
outside the previous anonymous-closure audit. Native timers now return the
existing Io variant with TimedOut; malformed received integer values retain
protocol classification through InvalidData, and local frame-length conversion
becomes invalid_input. Public error enums remain exhaustive with their existing
WebInspector six, Companion four and Notification five variants. Existing public
signatures/fields/root exports, dependencies/lockfile/features and classifier stay.
Returned variants, Display, immediate IO payload and safe correlations can change.
Migrate Timeout/Protocol or immediate get_ref matches to typed source/class
inspection. Original errors may contain private data; explicit native access
still follows the diagnostic policy. Fixed operation context and outer Debug stay
safe; automatic diagnostics do not expose original values.

WebInspectorError and CompanionError add `timeout_duration() -> Option<Duration>`.
Private deadline-issuer payloads retain the original Elapsed and the budget that
the former Timeout adapter reported. The getter accepts that service's legacy
Timeout variant or its private issuer within at most 32 typed sources. It never
formats errors or infers issuance from IO TimedOut/general Elapsed/category.
Generic IO timeout can therefore classify timeout while this getter returns
None. None does not prove the error is not a timeout. A startup report-write
deadline reports the original budget, while startup response/event stages can
report their remaining phase budget; the getter is not a fresh remaining-time
estimate or a retry/cleanup guarantee.

This issuer distinction preserves existing behavior: WebInspector page listing
consumes its own idle expiry as quiescence; two CLI discovery loops and two CDP
loops consume only those named deadlines. Underlying IO TimedOut still returns
an error. Legacy `start` deliberately maps response own-timeout/closed-stream
errors to Disabled and loses their original source. Its blocked identifier-write
deadline remains directly returned. `start_preserving_errors` keeps actual
startup native errors, including Elapsed under Io. Created remaining-zero
WebInspector/Notification Timeout markers stay source-free; no native cause is
invented. Request budgets, selectors, framing, success, retries and event order
stay unchanged.

Companion's listener still marks finished before returning its timeout and never
resumes a potentially partial frame. Its stored pending future/stream ownership
is unchanged; this does not prove task joins or remote release. Forwarding stop
still sets stopped/caches the response only after the existing response succeeds;
Drop's bounded best-effort stop is unchanged. Port parsing retains unsigned
precedence, type/zero guards and admitted range, with original conversion objects
for negative/oversized received values. Notification observation state updates
only after its successful send, and ProxyDeath/event/zero-budget behavior stay.

WebInspector explicit page ID precedence, PID delimiter policy and maximum frame
guard remain. The private frame-length converter was equivalently extracted
before baseline tests, still using the old eraser. The configured 16 MiB maximum
rejects before conversion/write; direct helper overflow exercises the native
conversion without a giant allocation or claiming that public frame path reaches
u32 overflow.

Production comparison passes after excluding native adapters, private helpers,
the equivalent frame converter extraction and the own-deadline predicate migration.
Inspector three, Automation five, model one, Notification two, client five,
wire two and Companion nine map_err calls remain in the compared regions;
Companion's rewritten port parser, issuer helpers/getters and listener arm are
reviewed separately. CLI bodies change only four deadline branches and removal
of the unused CDP import/rustfmt layout. Cargo files/lockfile, core root exports,
classifier and test_native are byte-identical. Logs/snapshot:
`/tmp/ios-a02-discovery-native-code-audit.log`,
`/tmp/ios-a02-discovery-native-source-diff.patch`.

| Check | Result |
| --- | --- |
| All-feature WebInspector / Companion / Notification library | 38 / 16 / 10 PASS; `/tmp/ios-a02-discovery-native-{web,companion,notification}.log`, terminal 0 |
| Corresponding no-default service feature library | 38 / 16 / 10 PASS; `/tmp/ios-a02-discovery-native-{web-minimal,companion-minimal,notification-minimal}.log`, terminal 0 |
| Final native discovery regression group | 12 PASS; `/tmp/ios-a02-discovery-native-after-final2.log`, terminal 0 |
| External WebInspector exchanges | 12 PASS; `/tmp/ios-a02-discovery-native-web-public-final.log`, terminal 0 |
| External public_contract all / no-default | 26 / 13 PASS; `/tmp/ios-a02-discovery-native-{public,public-minimal}.log`, terminal 0 |
| CLI WebInspector tests | 39 PASS; `/tmp/ios-a02-discovery-native-cli.log`, terminal 0 |
| Final core/CLI strict Clippy | PASS; `/tmp/ios-a02-discovery-native-clippy-final.log`, terminal 0 |
| Full validation with workspace .venv Python | First full PASS; `/tmp/ios-a02-discovery-native-full.log`, terminal 0 |

Twelve new internal tests and one external consumer use owned duplex/pending
streams and actual short Tokio timers, actual PID/page/port parsing faults,
bounded issuer traversal with a cyclic source whose Display panics, and exhaustive
public error families. Inspector blocked writes cover attach, unpaused attach and
command; Automation covers initial attach write and command. Later Automation
attach stages are source-reviewed, not independently fault-injected. Partial
write tests establish retained sources, without certifying connection reuse.
The existing external idle-read test still receives a later complete frame;
that fixture does not establish reuse after partial-frame consumption. The
controlled AsyncRead TimedOut fixture validates classification/issuer behavior,
not an actual operating-system timeout. Companion listener partial-header expiry
remains terminal, forwarding failed stop leaves local state/cache unset, and
Notification keeps observations/created-zero behavior. No real credentials,
device/daemon or Apple operation, remote cleanup or background-task join proof.

Before-fix corrected logs show **zero PASS/eleven FAIL**, terminal 101:
`/tmp/ios-a02-discovery-native-before-final.log` (ten internal) and
`/tmp/ios-a02-discovery-native-before-public-final.log` (one external).
The initial `/tmp/ios-a02-discovery-native-before.log` failed E0283 in a new
Dictionary key fixture before tests ran; replacing ambiguous into with to_string
fixed the fixture, not production. Post-fix native tests pass. The first external
WebInspector run had eleven PASS/one FAIL because a new assertion required exactly
20 ms, although the unchanged implementation supplies a slightly smaller remaining
phase budget; the final assertion checks a positive budget at most 20 ms and keeps
the later-frame check. Its log remains `/tmp/ios-a02-discovery-native-web-public.log`.
First strict Clippy rejected the now-unused CDP import and a test IO constructor;
the final code removes the import/uses IO::other. Original failure log:
`/tmp/ios-a02-discovery-native-clippy.log`. No production guard, budget, gate or
lint was weakened. Audit normalization also accounts for rustfmt's import layout.

Full gate: **1945 Rust/13 Python PASS, zero FAIL, two existing ignored**. Fmt,
check, strict workspace Clippy, MSRV, 66 feature builds/four groups, 7+7 doctests/
three snippets, C ABI/owned malformed/checked list, Python 3.12 wheel/sdist
rebuild/install, 53 tooling tests, 118 help records, 38 raw cases, five host
workflows/22 device NOT_RUN, 15 CLI and three backup contracts pass. No separate
Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug SHA256
`97defac193aecb4303b51abfa72587a46a40695626363ce08cf789600bbae7db`.
Source stayed frozen throughout full and afterward. About 15 GiB free at full
start and finish; no storage failure or cleanup.

Documentation closeout: 118 capability records, 136 local links across six
changed Markdown files (baseline 8a08f22), unchanged capability implementation/
maturity and historical evidence, and whitespace PASS. Logs:
`/tmp/ios-a02-discovery-native-docs-catalog.log`,
`/tmp/ios-a02-discovery-native-docs-links.log`. The seven-file documentation
commit leaves the tested source/full gate unchanged.

A02/class 3/overall goal remain active; class 4 has not started. The next read-only
anonymous scan finds 47 candidates in 21 service files, not 47 confirmed native
losses; formatted/.ok()/match-arm and created/optional outcomes require separate
review. Logs `/tmp/ios-a02-discovery-native-next-services.log` and
`/tmp/ios-a02-discovery-native-next-consumers.log` record the next scope. CrashReport
watch parse retries consume a named parser failure and require issuer-aware
migration; other services, DProxy textual observations, codecs/typed guards/
created outcomes, stream/JSON/callback/binding and L01/L03/A03/platform/device
conditions remain. All 31 tasks/subtasks and capability maturity stay. No push/
publication, history rewrite or reference checkout edits.
