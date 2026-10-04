# CLI diagnostics and partial CPU failures: host validation

Accepted 2026-10-04, A02/class 3 source
`5f259b8f90dd40f973085a00b401a37a62066ac1`, baseline
`d12449c810fe6564e0f3cfcd32ac0f15cef223db`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, workspace Python 3.12.13. Seventeen source files, 467 insertions and
103 deletions. [Public contract](../public-contract.md),
[error contract](../error-contract.md).

Sixty direct CLI eprintln sites now use the existing checked stderr helper.
Fifty-four propagate the original write/flush IO error. Six keep an already
failed operation primary: partial credential publication, accessibility cleanup,
two CDP bridge closures, screenshot client failure and sysmontap CPU read
failure. The helper retains its native IO and fixed write/newline/flush behavior.
Unchanged diagnostic arguments include the existing redaction decisions;
this change does not add a new sanitizer for arbitrary stderr content.

Existing successful stdout and diagnostic line bytes remain. Stderr remains
diagnostic output, including when final machine failures are selected. Status
writes occur at their existing positions and can follow output or effects;
they are not a universal preflight or rollback boundary. A report write failure
must not replace the original operation error. Final error reporting remains
best effort when stderr is unavailable, so a missing envelope is not success.

Sysmontap CPU previously broke out after a read error, attempted stop and
returned success even after partial output. It now attempts the same stop and
returns the original DtxError, even if reporting also fails. Normal count,
deadline and Ok(None) completion policies stay. Stop failures remain ignored;
stdout-write failure still returns at the existing point before stop. No new
cleanup timeout, acknowledgement, retry, rollback or safe-reuse promise.

The private CPU renderer retains finite two-decimal JSON/human formats and
locks stdout per record. JSON rejects NaN and either infinity before writing
that record, with source-free InvalidData/Protocol. Earlier rows remain.
Human formatting stays. The shared writer gains crate-only visibility with
an unchanged body, supporting this production renderer without a public API.

Unix source/destination path CString conversions retain the original owned
NulError through the existing IO InvalidInput and safe private context. Fixed
messages, input class, path policy and rename syscalls stay. Explicit source
access contains original path bytes and remains subject to the privacy contract.
Native NUL rejection occurs before rename; it is not an OS rename fault.

Four tests were added, three CLI and one core. The core test checks source and
destination NUL paths, original position/bytes and safe container Debug. The
Linux CLI test owns a test-binary child with global stderr at /dev/full, checks
native ENOSPC and IO classification before mDNS discovery, and bounds/reaps it.
The original eprintln implementation compiled but failed that test (0 PASS,
1 FAIL); the checked implementation passes. Parent stderr is unchanged.

The CPU tests use actual DTX/recursive archive encoding and SysmontapService
over joined owned duplex peers. One legal JSON row precedes actual read EOF;
an injected diagnostic PermissionDenied cannot replace the primary native
UnexpectedEof. The request sequence includes the stop frame. A count-limited
successful run retains the same row and stop behavior. Both peers complete
within one second. This proves local stream behavior, not remote teardown.
The non-finite fixture checks that a prior row survives without a bad new row.

Reinstating the old break-to-success policy and removing the JSON finite guard
in the extracted private implementation made both CPU tests fail (0 PASS,
2 FAIL). This is an old-behavior prototype control, not a build of the original
whole CLI. The final source was restored byte for byte. Earlier fixture
preparation found an ambiguous dictionary key, wrong hardcoded request IDs
and non-recursive sample encoding; these were corrected before acceptance and
are not behavioral negative controls or production failures.

After full, four actual ios binary children ran pair/pair wifi with human/JSON
error selection and stderr at /dev/full. Each returned exit 1 with zero stdout
within three seconds, at its first diagnostic before discovery. No real device,
pairing, credential or daemon action was performed. These checks do not prove
all sixty status sites execute, or delivery of a final envelope on broken stderr.

Whole-body comparison of fourteen CLI production files matches only the
reviewed replacements; the pair test is excluded from that production check.
All sixty diagnostic arguments match baseline apart from the CPU callback's
local borrow name. CPU extraction, primary failure and finite guard were
reviewed separately. Filesystem changes affect only the two source adapters.
Dependencies, classifier, root APIs/public consumer, DTX codecs, C/Python
sources and wire stay. Source snapshots remained frozen through all checks.

| Check | Actual result |
| --- | --- |
| CLI all features | 667 PASS, 1 existing ignored |
| Filesystem all / new test minimum | 4 / 1 PASS |
| Fmt and strict core/CLI Clippy | terminal 0 |
| First canonical full | terminal 0; 2082 Rust/13 Python PASS, 0 FAIL, 2 existing ignored |
| Actual CLI broken-stderr children | 4 PASS, exit 1 and empty stdout |

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. Fourteen Rust groups and
one Python group, 66 feature builds/four groups, 7+7 doctests/three snippets,
workspace fmt/check/strict Clippy/MSRV, C ABI/malformed/checked listing,
3.12 wheel/sdist rebuild/install, 53 tooling/118 help/38 raw/five host/22
device NOT_RUN/15 CLI/three backup contracts pass. No separate Chromium or
installed Python 3.9/3.13 rerun. Logs use `/tmp/ios-a02-cli-diagnostics-`:
before.log, after-pair.log, cpu-old-behavior.log, after-cpu-final.log,
narrow.log, cli/filesystem/minimal/clippy.log, code-audit.log, full.log,
binary-stderr.log and documentation audit.

Staged source diff SHA256:
`423b46a2737982713d06d837e25829a1d9530cdbc10ae2e6bb911e85411c6e85`.
Tested ios 0.1.13 debug SHA256:
`1f45befb8217367d540337f251b7b390feac03074ccbf2967f0114cbad926d4b`.
After narrow checks stopped, official cargo clean -p ios-cli removed 6336 files,
16.8 GiB of CLI artifacts; no manual target deletion. Full started with about
30 GiB free and finished with 27 GiB, with no storage failure or duplicate full.

Eight documents were reviewed separately: seven Markdown files/178 local
links, whitespace and exact source/binary checks pass. The catalog retains
all 118 capabilities, implementation/maturity and historical evidence, adding
one accepted host entry.

A02/class 3/overall goal remain active; class 4 has not started. All 31 tasks
and subtasks stay. Next: remaining CLI native/elapsed adapters, typed decisions
and finite stream/binding acceptance review. L01/L03/A03 and platform/device
conditions remain. This gate does not certify physical services, remote cleanup
or maturity. No reference edit, push/publication or history rewrite. The
[preceding connection gate](connection-native-sources-2026-10-04.md) is retained.

Follow-up: [CLI native source acceptance](cli-native-sources-2026-10-04.md)
preserves service/parser/file/elapsed objects and typed cancellation. This
diagnostic source commit and its accepted evidence remain unchanged.
