# A02 archive and backup metadata sources — 2026-10-02

Implementation `dcdda80b2a13e7d3d18f713e20e38be373674343`, five source files.
Branch `feat/device-service-updates-20260927`, starting clean HEAD `c5f338a`.
Linux x86_64, Rust 1.97.1, MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Twenty-two existing source adapters retain their original error objects: the
private NSKeyedArchiver plist adapter; all 18 anonymous Manifest adapters; and
three common SQLite, crypto and MBDB adapters used by their existing call sites.
The private ArchiveError::Plist payload becomes native plist::Error with source;
ArchiveError has no public root export or public signature exposure. Public
Mobilebackup2Error retains its four variants, root APIs/signatures/result fields
and feature definitions. Existing Protocol-to-Io failures now carry bounded
private fixed context and the original object. No dependency or binding change.

The Manifest closures retain 12 TryFromIntError conversions, ParseIntError for
ProductVersion, two StripPrefixError checks, the original identifier validation
error, ArchiveError and FromUtf8Error for symlink bytes. Known metadata/path/
integer/UTF-8 failures remain protocol through InvalidData. Archive wrappers use
Other with cached native classification, preserving TooDeep as resource_limit
instead of losing it to Protocol. Validation-created guards without an original
error keep their previous outcomes; no artificial backend source is invented.

Crypto IO retains the original CryptoError and nested filesystem IO/OS code and
class. Known malformed keybag/manifest/payload or ambiguous invalid password/
wrapped-key failures remain protocol; the opaque Crypto marker becomes unknown.
This retains the original existing typed crypto/MBDB errors, without recovering
backend/UTF-8 causes already discarded inside those private modules. MBDB's
fixed validation errors remain protocol. Subsequent
[inner source validation](backup-inner-sources-2026-10-02.md) retains native
AES-KW/cipher/conversion and MBDB UTF-8/integer causes; this historical gate is unchanged.

With backup2-manifest, SQLite native codes are classified without reading prose:
permission/authorization denied; busy/locked/constraint conflict; memory/disk/
size resource_limit; read-only precondition; corrupt/not-a-database protocol;
system IO as io. Unknown codes stay unknown. CannotOpen is not guessed to be
NotFound or permission; OperationInterrupted does not acknowledge caller
cancellation. Native code/extended code/message objects survive. Manifest column
type/range/UTF-8 failures retain protocol; other conversion sources remain traversable.

Migrate affected Protocol matches to class/source observations. Display prefixes,
dynamic path/identifier context, correlations and categories can change; fixed
phase labels replace adapter-created prose. Earlier strings cannot restore native
objects. Raw source/native Display/Debug is private data: FromUtf8Error owns the
original target bytes, and SQLite sources may contain SQL/path metadata. Context
and outer diagnostic Debug omit these payloads. Machine output schema stays.

Production comparison normalizes only error adapters/helpers and whitespace:
all 53 existing Manifest map_err calls remain. SQL, bounds, depth 64, path/symlink
policy, format/version selection, encryption parameters/Zeroizing storage,
transactions, staging/replacement/rollback, retries and successful metadata/
extraction behavior stay. DTX/instruments' raw/optional archive fallbacks remain.
No protocol, timeout, retry, lifecycle, trust or CLI schema change is added.

| Check | Result |
| --- | --- |
| Core no-default archive / minimal backup2-manifest Manifest | 21 / 27 PASS; `/tmp/ios-a02-backup-sources-{archive-basic,manifest-basic}.log`, terminal 0 |
| Core all-feature backup2 | 78 PASS, one existing PBKDF2 ignored; `/tmp/ios-a02-backup-sources-backup-full.log`, terminal 0 |
| External public_contract all / minimal backup2-manifest | 21 / 12 PASS; `/tmp/ios-a02-backup-sources-public{,-minimal}.log`, terminal 0 |
| Core/CLI strict Clippy / production comparison | PASS; `/tmp/ios-a02-backup-sources-{clippy,code-audit}.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-backup-sources-full.log`, terminal 0 |

Eleven new core fixtures and one external root consumer cover actual malformed
plist, cyclic archive/depth, metadata Mode/Size conversion, ProductVersion overflow,
legacy magic, keybag parser, native filesystem IO adapter, invalid symlink UTF-8,
and public list/extract faults and valid payload/result fields. Failed metadata
admission creates no extraction directory; malformed symlink text creates no
link. Owned UUID directories clean up through fixture Drop. No existing backup,
real credentials, device, daemon or Apple endpoint is used. This does not prove
transaction rollback after every failure, stream reuse or remote cleanup.

Actual SQLite fixtures cover not-a-database, missing schema, unique constraint
with preserved extended code, exclusive lock/busy and read-only failures. An
exclusively owned in-memory connection uses a static authorizer callback to deny
SELECT, then a 64-byte BLOB limit rejects zeroblob(65) before allocation. These
test-only FFI calls have no borrowed callback data and do not alter production
limits or busy handling. CannotOpen/Interrupted with misleading prose use typed
native-code fixtures, not real OS permission/cancellation operations. Not every
mapped SQLite code, allocator, filesystem step or architecture-dependent
conversion is independently fault-injected; invalid Mode/Size are actual conversions.

Corrected pre-fix fixtures: **1 PASS / 7 FAIL** across eight initial fixtures,
`/tmp/ios-a02-backup-sources-before-final-{archive,manifest}.log`, terminal 101.
The earlier broad run (`...-before.log`, 17 PASS/8 FAIL) included an incomplete
fixture missing required Info.plist/Status.plist; a focused debug run also failed.
Those failures did not establish the intended native metadata regressions.
Adding the required owned files enabled the final pre-fix run and valid success
fixture. Original logs remain. Three additional core fixtures and the external
consumer were then added. No production precondition, gate or lint was weakened.

Full gate: **1901 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three snippets, C ABI/owned malformed/checked-list, 3.12 wheel/sdist
rebuild/install, 53 tooling tests, 118 help records, 38 raw diagnostic cases,
five host workflows/22 real-device NOT_RUN, 15 CLI and three backup contracts
pass. No separate Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug
SHA256 `b1f1292e8968cd664b18ecaef8e50f796e7e9268be7cd60c2c5b41ab7a40f5df`.
Source stayed frozen through full and afterward. No storage failure/cleanup;
about 23 GiB free before and after full.

Documentation closeout: capability catalog 118 records, 100 local Markdown links
across five changed Markdown files (baseline `c5f338a`), and whitespace PASS.
Logs: `/tmp/ios-a02-backup-sources-docs-catalog.log` and
`/tmp/ios-a02-backup-sources-docs-links.log`. Source and the accepted full gate
remain unchanged.

A02/class 3/overall goal remain active; class 4 has not started. The subsequent
inner crypto/MBDB slice is linked above. Host filesystem and archive integer sources, other codecs/typed guards/
created outcomes/stream/JSON/callback/binding and L01/L03/A03/platform/device
conditions remain. All 31 tasks/subtasks and maturity remain. No push/publication,
history rewrite or reference checkout changes.
