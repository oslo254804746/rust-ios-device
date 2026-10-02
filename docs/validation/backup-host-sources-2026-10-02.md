# A02 backup filesystem and archive integer sources — 2026-10-02

Implementation `47ed0d5f2649ce4ed65af54ca9b5444246c6e666`, five source files,
starting clean HEAD `2b26398` on `feat/device-service-updates-20260927`.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Seven previously erased native causes now survive: five backup host StripPrefixError
adapters, the missing-required-file filesystem error and one archive integer
conversion. A further JoinError adapter already retained its object, but now uses
the existing fixed context/category snapshot so a panic is unknown rather than
ordinary IO. Native cancelled JoinError keeps cancelled; that category does not
acknowledge termination of a running blocking worker or device cleanup.

Outside-root paths use existing Mobilebackup2Error::Io with InvalidData/protocol.
Missing Info.plist/Manifest.plist/Status.plist now use Io with native NotFound,
original nested IO and OS code, instead of Protocol prose. This is host not_found,
not device_not_found or service_missing. The outer IO context's raw_os_error is
not the original OS code; inspect the nested native IO through Error::source.
Fixed phases replace adapter-created path text. Display, returned variant,
correlations and the missing-file/panic categories can change. Callers migrate
Protocol matches and immediate get_ref downcasts to class/source traversal.
Public Mobilebackup2Error retains four variants, root APIs/results and features.

The crate-private archive error adds Integer(#[source] TryFromIntError), preserving
the three other variants. It has no public root export/signature exposure. Actual
unsigned values above i64::MAX remain rejected as protocol, with the original
conversion object instead of Invalid numeric prose. Values through i64::MAX and
signed primitives keep their existing decoding. The classifier adds this one
private variant to its existing protocol arm. No public error enum changes.

Context/outer Debug omit private path, native and panic payloads. Raw source/Debug
access remains private data; runtime panic hooks are unchanged. Validation-created
guards without a native cause stay. Path/symlink/macOS-alias/Windows normalization,
metadata caps, incremental policy, staging/commit/copy/remove/list behavior,
spawn_blocking scheduling, decoder depth and optional UID/raw fallbacks stay.
Dependencies, lockfile, public features/exports and Manifest production are
byte-identical. Production comparison normalizes only changed adapters/helper,
missing-file branch payload, private Integer variant and whitespace; all 14 host
map_err calls remain. Log: `/tmp/ios-a02-backup-host-code-audit.log`.

| Check | Result |
| --- | --- |
| Core no-default archive decoder / host filesystem | 9 / 20 PASS; `/tmp/ios-a02-backup-host-{archive-basic,host-basic}.log`, terminal 0 |
| Minimal backup2-manifest / all-feature backup2 | 28 / 87 PASS, one existing PBKDF2 ignored in all-feature backup2; `/tmp/ios-a02-backup-host-{manifest-basic,backup-full}.log`, terminal 0 |
| External public_contract all / no-default | 23 / 12 PASS; `/tmp/ios-a02-backup-host-public{,-minimal}.log`, terminal 0 |
| Core/CLI strict Clippy | PASS; `/tmp/ios-a02-backup-host-clippy.log`, terminal 0 |
| Full validation with the workspace .venv Python | First full PASS; `/tmp/ios-a02-backup-host-full.log`, terminal 0 |

Four new internal tests and one external consumer establish actual layout/copy
source/copy destination/list prefix failures without outside-path creation, all
three required-file missing failures with native OS codes and restored valid
admission, binary archive integer overflow and public metadata list/extract
rejection without output creation. The external root application reader proves
valid success then missing Status.plist rejection with original IO. Existing
unsigned boundary and worker IO/panic tests now check type/class/source; timer
progress and existing path/copy/staging fixtures still pass. Owned UUID fixture
directories use Drop cleanup. Copy-parent conversion is source-reviewed rather
than independently injected; worker cancellation, every OS/allocator/architecture
fault, running-worker termination, rollback stage and remote cleanup are not
certified. No existing backup, real credentials, device/daemon or Apple operation.

Corrected before-final logs: **17 PASS/6 FAIL**, four commands each terminal 101,
`/tmp/ios-a02-backup-host-before-final-{archive,host,manifest,public}.log`.
The six failures establish native loss or the known wrong panic/missing-file
category. Earlier before logs failed compilation due to new test-only lifetime
and dictionary-key inference issues; those do not establish native regressions.
They were corrected before repeating the baseline. Logs remain. Existing unsigned
and private exhaustive negative test matches were migrated to Integer; no guard,
limit, successful behavior or lint was weakened.

Full gate: **1913 Rust/13 Python PASS, zero FAIL, two existing ignored**. Fmt,
check, strict workspace Clippy, MSRV, 66 feature builds/four groups, 7+7 doctests/
three snippets, C ABI/owned malformed/checked list, Python 3.12 wheel/sdist
rebuild/install, 53 tooling tests, 118 help records, 38 raw cases, five host
workflows/22 device NOT_RUN, 15 CLI and three backup contracts pass. No separate
Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug SHA256
`c6ac701899c416c8d032e6ce46f6e6f8bd6d1c574b306720ee7f65ab7dabcf1e`.
Source stayed frozen through full and afterward. Before this slice's builds,
all Cargo/validation/maturin processes had stopped; cargo clean -p ios-core
removed 38.9 GiB of generated crate cache, freeing about 11 to 48 GiB. Log:
`/tmp/ios-a02-backup-host-cache-clean.log`. No storage failure; about 45 GiB free
at full start and 29 GiB after.

Documentation closeout: 118 capability records, 105 local links across five changed
Markdown files (baseline 2b26398), and whitespace PASS. Logs:
`/tmp/ios-a02-backup-host-docs-catalog.log` and
`/tmp/ios-a02-backup-host-docs-links.log`. Source and full gate stay unchanged.

A02/class 3/overall goal remain active; class 4 has not started. Read-only next
audit confirms three OPACK UTF-8 erasers and XPC framing/integer/deadline adapters,
recorded in `/tmp/ios-a02-backup-host-next-codec-audit.log`. Optional authenticated
M6 info, UID and DTX/instruments raw fallbacks keep their existing behavior.
Other typed guards/created outcomes, stream/JSON/callback/binding and
L01/L03/A03/platform/device conditions remain. All 31 tasks/subtasks and capability
maturity stay. No push/publication, history rewrite or reference checkout edits.

Follow-up: implementation `fcb4cb5dfde7637d74581b72647b97c6b847bd2f` retains
32 OPACK/XPC/identity native sources; [XPC evidence](xpc-native-sources-2026-10-02.md)
records its separate 1921 Rust/13 Python full gate. This page's historical 1913
gate remains. DProxy's textual projection, other native adapters, created guards,
streams/bindings and lifecycle/platform/device conditions still remain.
