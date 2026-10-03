# Service input sources: host validation

Date: 2026-10-03. A02/class 3 source:
`ab097900523e91f93e54249c42c214a7e1349ee6`, branch
`feat/device-service-updates-20260927`, clean baseline
`a2b69cc2777fc81b03b528ff4df375ca6f5e907e`. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, workspace Python 3.12.13.
This is host source/compatibility acceptance, not a device workflow.

Eleven native errors were discarded into Protocol/GpxParse text or None.

| Owner | Original native boundaries | Current representation |
| --- | --- | --- |
| AppService | PID TryFromIntError; fixed empty binary plist Error | Existing Xpc(Io InvalidInput) |
| Testmanager | Test PID TryFromIntError | Existing DtxError::Io InvalidInput |
| ZipConduit | Entry length/count usize-to-u64; ZIP u32/u16 and plist i64 | Existing Io InvalidInput |
| SimLocation | XML DeError; previous/current RFC3339 Parse | Existing Io InvalidInput |

Seven integer, one plist, one XML and two time parser causes survive. Context
is fixed/private. Existing AppServiceError/DtxError/ZipConduitError/
SimLocationError remain exhaustive with 2/3/5/2 variants. No new public type,
method, enum field, dependency or feature. PID/ZIP classification migrates from
protocol to invalid_input; GPX keeps invalid_input while GpxParse becomes Io.
Returned variants/Display/immediate payload/safe diagnostic correlation may
change. Use typed classes and bounded source traversal. Explicit native data
follows the [error contract](../error-contract.md).

Eleven source files: five production files, four new internal test modules,
the external public-contract consumer and an existing Testmanager integration
consumer. Public signatures/root/fields and shared classifier/context/DTX/H2/
XPC codecs stay. The private fixed binary-plist writer was equivalently extracted
before the old-adapter control; production still writes an empty dictionary into
Vec, with the original exact 42 bytes. Four whole extracted production files
match after native callbacks, private static phase lifetimes and the separated
record-count conversion are normalized. Record-count checked_add and all
other arithmetic/validation guards stay. Existing ZIP decoder/read mappings
are byte-identical. Two old AppService unit assertions and one integration
assertion migrate to the new returned variant.

App feature admission remains before request construction. PID conversion stays
before XPC calls or Testmanager control lookup. GPX builds its whole route before
replay writes; coordinate strings, optional/paired timestamp behavior, delay
rounding, negative-delta zero clamp and wire remain. ZIP init/metadata/header/CRC/
entry/terminator/progress order stays. Header no-write evidence is scoped to its
private encoder; a public installation input fault can occur after earlier
stream writes. No new parser/allocation/transfer bounds, timeout, cancellation,
cleanup, retry, rollback or reuse guarantee. Existing location string length
casts and lifecycle work remain separate.

Final source diff SHA256:
`feffb2e4c54fe30f6822799cd5a66ab49ea0cdc65084df0eeb44d35c290df242`.
All eleven files, including new files, staged before final full. Logs share
`/tmp/ios-a02-service-input-`: code-audit-final.log, source-diff.patch,
original-{app,zip,sim,test}.rs and baseline counterparts. The initial ten-file
diff before integration migration had SHA256
`0370aa7472fb8e54873bc805e90c1b5b23486c0498ee0c33beaaed93921fb4df`
and is retained as source-diff-first.patch. The encoder audit initially assumed
unchanged formatter punctuation; callback/trailing-comma normalization fixed
the extraction check without changing source.

Nine internal and two external tests were added. Actual integer overflows and
XML/time parsing faults preserve native objects; both PID request builders and
both time positions are exercised. A private writer injects an actual captured
owned file-open NotFound error: plist and nested IO/raw OS code survive, while
the encoder wrapper class is input. This is not a production Vec IO fault or
actual filesystem write. Fixed-empty plist success bytes pass under the old
adapter before its source assertion. u32/i64 tests use private scalar/metadata
helpers without giant allocations; u16 exercises a private 65536-byte filename
before header writes. The two usize-to-u64 failures cannot occur on this 64-bit
host and are source-reviewed. No fabricated overflow or physical acceptance.

Owned duplex tests check actual EOF and zero bytes after invalid GPX replay,
invalid Testmanager PID with absent control, and the migrated integration with
both control/session present. Joins/reads have one-second bounds. They prove
local fixture behavior, not remote task cleanup. External consumers actually
call public GPX/authorization APIs, construct/destructure GpxRoutePoint and
exhaustively match the original two/three-variant families. AppService's full
physical XPC invocation is source-reviewed; no device/control startup is run.

Negative control: before-final.log **0 PASS / 9 FAIL**, before-public.log
**0 PASS / 2 FAIL**, all at missing native source/old returned variants. The first
before.log also fails all nine; both timestamp inputs were gathered before
assertions in before-final. No compile failure or invalid fixture is counted.
One initial apply_patch context mismatch aborted before source changes; the
formatted encoder was read and the patch reapplied. No guard/lint/budget was
weakened.

| Check | Result / log |
| --- | --- |
| New native contracts | 9 PASS / after.log |
| Apps service unit scope | 57 PASS / apps.log |
| Testmanager unit scope | 16 PASS / test.log |
| SimLocation unit scope | 8 PASS / gpx.log |
| All-feature public consumer | 34 PASS / public.log |
| Minimum apps,testmanager | 7 native / 17 public PASS; AppService needs tunnel |
| Minimum apps,testmanager,tunnel | 9 native / 18 public PASS |
| Migrated Testmanager integration | 19 PASS all-feature and 19 minimum; test-integration logs |
| Strict Core Clippy | exit 0 / clippy.log |
| Canonical full, initial | terminal 1; Rust stage exit 101 / full-first-failed.log |
| Canonical full, corrected | terminal 0 / full-final.log |

The first full failed one existing integration test (18 PASS/1 FAIL in that
group) still requiring a Protocol message for oversized PID. That consumer was
updated to assert Io InvalidInput, actual TryFromIntError and zero bytes on both
owned streams; production code stayed. Both integration feature profiles passed
before complete full rerun. The failure and initial source hash remain visible;
this is not a first-pass full claim.

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. Final **2041 Rust/13 Python
PASS, zero FAIL, two existing ignored**; fourteen Rust groups and a separate
Python host group (full-counts.log). Fmt/check/strict workspace Clippy/MSRV,
66 feature builds/four groups, 7+7 doctests/three snippets, C ABI/malformed/
checked list, Python 3.12 wheel/sdist rebuild/install, 53 tooling tests,
118 help records, 38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI
and three backup contracts pass. No separate Chromium or installed 3.9/3.13
rerun. Tested ios 0.1.13 debug SHA256
`77a948d729afacab1a968d5de9dd34ad116eaa961e50539ac25f82e752aee43b`.

After completed narrow handles, free space was 1.9 GiB. Only official cargo clean
ran, reporting 82403 files/98.4 GiB; free space became about 94 GiB. Source and
credentials/references stayed. The tunnel minimum was rebuilt, first full then
the corrected full ran; final free space about 69 GiB, with no storage failure or
further cleanup. clean-before.log/clean.log and the final source audit record
this sequence. Sources stayed frozen throughout each full; only the identified
integration test changed after the first validator was terminal. Documentation
does not change accepted source.

The 118 capability records retain implementation/maturity and historical
evidence, adding one host entry. Seven changed Markdown files have 161 valid
local links (baseline a2b69cc), with whitespace PASS. Audits use
docs-links.log/docs-catalog.log;
eight documents are committed separately. A02/class 3/overall goal stay active,
class 4 has not started, and all 31 tasks/subtasks remain. Guarded AFC/DTX/file/
symbol conversions and DProxy event/string/stderr observations remain; read-only
next-guarded-sources.log and next-dproxy-observations.log record actual placement
and local zerocopy owned-source support. Other typed guards/created outcomes/
stream/JSON/callback/binding and L01/L03/A03/platform/device conditions stay.
No physical app/test/install/location workflow, daemon/credentials/Apple action,
reference edit, push/publication, history rewrite or maturity upgrade. See
[public contract](../public-contract.md).

Follow-up: [DProxy decoder observations](dproxy-decoder-observations-2026-10-03.md)
accept additive original errors, typed local decisions and partial events at
source `a37a80b6eb7be4cc5f6d096e45eca1a65930e133`. Recorder/CLI adoption and
checked stderr remain pending, as do the guarded converter and other A02 tasks.
This follow-up does not alter the preceding source/gate or capability maturity.
