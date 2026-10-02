# A02 encrypted backup and legacy index sources — 2026-10-02

Implementation `7e5f174169964b713257c037c395312f841dc49c`, five source files,
starting HEAD `df9649e` on `feat/device-service-updates-20260927`.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Twelve private crypto adapters now retain their original objects: four keybag
tag/length/integer conversions, class identifier conversion, AES-KW initialization
and unwrap, AES-CBC initialization and four stream count conversions. Fixed
context uses the existing CryptoError::Io; its six variants stay. Malformed
keybag/class identifiers and failed key unwrap use InvalidData/protocol. Opaque
cipher initialization/count failures use Other with cached unknown rather than
invented IO evidence. Native read/write errors retain their original IO kinds.

The existing optional aes-kw 0.2.1 dependency enables its std feature so its native
Error implements std::error::Error. Version, Cargo.lock and public feature names
stay. This feature also enables the dependency's alloc APIs; production continues
using the same slice-based unwrap API. Local primary sources inspected were that
resolved dependency's Cargo.toml and src/error.rs, plus crypto-common 0.1.7 for
InvalidLength's existing Error implementation. Feature resolution is recorded in
`/tmp/ios-a02-backup-inner-crypto-features.log`. No AES std feature is introduced.

Three MBDB conversions retain FromUtf8Error or TryFromIntError: parsed strings,
property counts and serialized field lengths. The private MbdbError tuple becomes
a Clone error enum with Invalid/Utf8/Properties/Length variants and native sources.
Private Eq/PartialEq are removed because the native integer error lacks them;
there were no consumers of these private traits or the old tuple field. Outer
Debug uses the existing redaction boundary. Public Mobilebackup2Error still has
four variants, with unchanged root signatures/results. Its existing MBDB adapter
keeps these errors protocol. FromUtf8Error explicitly exposes original private
metadata bytes; raw source/Debug access is not an automatic diagnostic surface.

Display prefixes/correlations change where fixed phases replace discarded causes.
AES-KW integrity failure remains protocol, and does not prove that a supplied
password rather than wrapped data is wrong. The existing public encrypted-flow
fixture consequently checks native IntegrityCheckFailed and class rather than
matching password prose. Validation-created guards retain their existing outcomes
and have no fabricated source. MBDB None still encodes ffff; lengths 0..65534 are
admitted, 65535 remains the reserved sentinel guard and larger lengths fail before
append with the original conversion error.

Production comparison against df9649e passes after normalizing only error adapters,
private error representation/helpers, equivalent map-closure braces, trailing call
commas and whitespace, plus reviewing the equivalent u16 admission block. Manifest
production is byte-identical. Cipher algorithms/parameters, PBKDF, wrapping, CBC,
Zeroizing, bounds, nullable fields, record/property order, file identifiers and
successful parsing/encoding remain. No binding, wire, retry or lifecycle change.
Comparison log: `/tmp/ios-a02-backup-inner-code-audit.log`.

| Check | Result |
| --- | --- |
| Core backup2 minimal / all features | 84 / 84 PASS, one existing PBKDF2 ignored; `/tmp/ios-a02-backup-inner-core-{basic,full}-final.log`, terminal 0 |
| External public_contract all / minimal backup2-manifest | 22 / 13 PASS; `/tmp/ios-a02-backup-inner-public{,-minimal}-final.log`, terminal 0 |
| Core/CLI strict Clippy | PASS; `/tmp/ios-a02-backup-inner-clippy-final.log`, terminal 0 |
| Full validation with the workspace .venv Python | First full PASS; `/tmp/ios-a02-backup-inner-full.log`, terminal 0 |

Six new internal tests and one external root consumer establish actual AES-KW
integrity failure, CBC read/write faults, MBDB invalid UTF-8/property overflow/
field overflow and public legacy listing rejection with native UTF-8 bytes.
The existing modern encrypted patch/unback fixture retains its success, wrong
password rejection, no output payload and existing source assertions. Separate
actual invalid cipher constructors and slice/integer conversions exercise the
shared adapter; fixed-width production conversions are source-reviewed, not
claimed fault-injected on this host. Owned UUID directories clean up through Drop.
No existing backup, real credentials, device/daemon or Apple endpoint is used.
This does not prove every native/allocator/architecture failure, all rollback
stages, connection reuse, retry safety or remote cleanup.

Corrected pre-fix fixtures: 0 PASS/4 FAIL (one AES-KW, three MBDB),
`/tmp/ios-a02-backup-inner-before-final-{crypto,mbdb}.log`, terminal 101.
Earlier before logs failed compilation due to a test-only Option borrow typo,
and do not establish native regressions. First post-fix minimal run was 81 PASS/
1 FAIL/1 ignored because the existing encrypted-flow fixture expected discarded
password/wrapped prose; its assertion was migrated to native type/class. No
production guard or lint was weakened. Failure logs remain. Exactly one synthetic
fixture left by that failed assertion was verified by PID/nonce/mtime, expected
files/payload and absence of symlinks, then removed; no unrelated temporary path
was touched. Log: `/tmp/ios-a02-backup-inner-fixture-cleanup.log`.

Full gate: **1908 Rust/13 Python PASS, zero FAIL, two existing ignored**. Fmt,
check, strict workspace Clippy, MSRV, 66 feature builds/four groups, 7+7 doctests/
three snippets, C ABI/owned malformed/checked list, Python 3.12 wheel/sdist
rebuild/install, 53 tooling tests, 118 help records, 38 raw cases, five host
workflows/22 device NOT_RUN, 15 CLI and three backup contracts pass. No separate
Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug SHA256
`ee8bc1e5e2e86fe323e98b840abe6a8b2d99ba002b49bb26c1432b8dfacd6a5d`.
Source stayed frozen through full and afterward. No storage failure/build cleanup;
free space was about 20 GiB before full and 11 GiB after.

Documentation closeout: 118 capability records, 103 local links across five changed
Markdown files (baseline df9649e), and whitespace PASS. Logs:
`/tmp/ios-a02-backup-inner-docs-catalog.log` and
`/tmp/ios-a02-backup-inner-docs-links.log`. Source and full gate stay unchanged.

A02/class 3/overall goal remain active; class 4 has not started. Host filesystem
and archive integer sources, optional codec fallbacks, typed guards/created
outcomes, stream/JSON/callback/binding and L01/L03/A03/platform/device conditions
remain. All 31 tasks/subtasks and capability maturity stay. No push/publication,
history rewrite or reference checkout edits.
