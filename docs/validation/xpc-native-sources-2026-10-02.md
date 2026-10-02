# A02 OPACK and XPC native sources — 2026-10-02

Implementation `fcb4cb5dfde7637d74581b72647b97c6b847bd2f`, twelve source files,
starting clean HEAD `2a9814e` on `feat/device-service-updates-20260927`.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Thirty-two previously erased native causes now survive: three OPACK UTF-8,
twelve XPC message UTF-8/integer/slice, four H2 integer, three XpcClient deadline,
seven RSD slice/deadline and three identity UTF-8/deadline/environment adapters.
The initial read-only audit found 31 anonymous map_err adapters; implementation
also found the non-Unix environment .ok() eraser. Original Utf8Error,
TryFromIntError, TryFromSliceError, Tokio Elapsed and VarError objects are retained.
Validation-created guards have no invented backend cause.

Public XpcError retains its four variants and H2Error its two variants. Native
failures use their existing Io variants and the existing fixed context/category
snapshot. OPACK's private InvalidUtf8 now carries its native Utf8Error; its other
four variants and protocol category stay. The classifier only updates that
private payload pattern, retaining its existing 32-source traversal bound.
No dependency/version/lockfile, public feature, root export or binding changes.

The private checked_xpc_body_len result becomes io::Result. A private source
wrapper retains both the declared u64 and the original conversion error on a
narrow host; the limit guard becomes InvalidData IO with its same declared-length
message. Conversion precedes the same admission limit and any body read/allocation.
Eight consumers adapt: message two, RSD three, H2 ready-message one, manual pairing
one and DProxy one. **DProxy still converts this result to String at its existing
textual diagnostic-event boundary.** Its two slice erasers and decoded-error
projection remain pending; this slice does not supply structured DProxy errors.
The pairing adapter now retains the nested IO rather than creating Xpc prose;
its protocol category stays, while returned variant/Display change.

Local XPC encoding length conversion changes from protocol to invalid_input;
received framing/UTF-8/integer errors remain protocol. Non-Unix COMPUTERNAME
NotPresent retains not_found and the native VarError; NotUnicode now uses
InvalidData/protocol instead of being collapsed into not_found. Empty successful
environment values still use the existing source-free not_found guard. Unix
gethostname syscall, 1024-byte buffer, NUL and empty-value checks stay.

Migrate affected Tls/Protocol/Xpc or immediate get_ref matches to class/source
traversal. Fixed phases replace endpoint/count/field prose; Display prefixes and
safe correlations can change. Context/outer Debug omit native private payloads.
Explicit raw source/Display/Debug access remains private data, including the
environment OsString. Machine diagnostics do not gain native raw payloads.

H2's two fatal count conversions retain native errors **and** set the same
first-error-only connection poison before returning, without window mutation or
queued writes. Later connection guards still return the cached protocol phase;
they do not reconstruct the original native cause. The other two window
conversions remain nonfatal as before. Frames, windows, buffering, limits,
tombstones, EOF, scheduling and stream behavior stay.

A shared private Tokio timeout adapter retains actual Elapsed for XpcClient's
three stages and RSD's existing timeout helper. Their existing durations and
separate stage budgets stay, including the fifteen-second XPC connect budget;
connector IO, success, reconnect factory ownership and polling flow stay.
Identity's bounded command retains Elapsed with its existing duration, output
cap, kill_on_drop and explicit error cleanup. This introduces no new L01 budget,
retry, remote cancellation or cleanup acknowledgement.

The read-only production audit passes for OPACK/H2/message/RSD/pairing adapters,
DProxy's one result adaptation, XPC helper/error family, existing H2 mapping,
classifier and dependencies/features/exports. Client timer consolidation and
identity parser/environment extraction are structurally reviewed, not an
automated equality claim. Log: `/tmp/ios-a02-xpc-native-code-audit.log`; snapshot:
`/tmp/ios-a02-xpc-native-source-diff.patch`. Initial audit-helper assertions missed
comments/trailing commas, generic function syntax and RSD's two test attributes;
the artifact normalizer was corrected, without changing production for the audit.
Wire values/order, success, parser depth/bounds and optional authenticated M6
info/UID/DTX/instruments raw compatibility fallbacks stay.

| Check | Result |
| --- | --- |
| Core no-default library | 226 PASS; `/tmp/ios-a02-xpc-native-basic.log`, terminal 0 |
| Final all-feature XPC tests | 94 PASS; `/tmp/ios-a02-xpc-native-xpc-full-final-env.log`, terminal 0 |
| All-feature pairing transport / DProxy | 18 / 12 PASS; `/tmp/ios-a02-xpc-native-{pairing,dproxy}.log`, terminal 0 |
| External public_contract all / no-default | 24 / 13 PASS; `/tmp/ios-a02-xpc-native-public{,-minimal}.log`, terminal 0 |
| Final core/CLI strict Clippy | PASS; `/tmp/ios-a02-xpc-native-clippy-final-env.log`, terminal 0 |
| Full validation with workspace .venv Python | First full PASS; `/tmp/ios-a02-xpc-native-full.log`, terminal 0 |

Seven new internal tests and one external root consumer establish actual
malformed UTF-8 in all three OPACK forms and XPC strings/keys, native local integer
overflows without giant allocation, and H2 fatal conversion faults through its
private helpers over owned duplex. H2 checks poison, unchanged connection window
and no queued updates; these are helper injections rather than remote DATA-cap
or device-lifecycle acceptance. Hostname UTF-8/empty/success, shared one-millisecond
Elapsed with local future Drop, unchanged inner BrokenPipe and successful result
are exercised. Existing RSD short-stage and owned busy-shell timeout tests now
check native Elapsed. Existing bounds, length rejection and declared-u64 tests
retain their checks while adapting typed results.

The environment test reads an actual missing UUID key without ambient writes,
then passes a typed NotUnicode containing owned invalid OsString data and checks
the same native value survives. Unix invalid bytes are exercised here; Windows
UTF-16 and actual Windows COMPUTERNAME runtime are NOT_RUN. On other platforms
the fallback is only a typed fixture. Linux /bin/sh command evidence does not
certify macOS /usr/libexec/remotectl. Production public TCP dial/initialization/
reconnect fifteen-second expiry and every narrow-host conversion were not
independently injected. No real credentials, device/daemon or Apple operation;
no remote cleanup, reuse or retry certification.

Corrected baseline regressions: **0 PASS/7 FAIL**, each command terminal 101,
`/tmp/ios-a02-xpc-native-before-{opack,message,timeout,identity}.log` and
`/tmp/ios-a02-xpc-native-before-final-{public,h2}.log`. An earlier external test
used the private message module and failed compilation (E0603); it was corrected
to the existing root codec exports before reproducing the baseline. That compiler
error is not evidence of native-source loss. Logs remain. Post-fix narrow checks
pass; the final environment fixture was refined to use actual invalid OS data.
No production guard, limit, feature gate or lint was weakened.

Full gate: **1921 Rust/13 Python PASS, zero FAIL, two existing ignored**. Fmt,
check, strict workspace Clippy, MSRV, 66 feature builds/four groups, 7+7 doctests/
three snippets, C ABI/owned malformed/checked list, Python 3.12 wheel/sdist
rebuild/install, 53 tooling tests, 118 help records, 38 raw cases, five host
workflows/22 device NOT_RUN, 15 CLI and three backup contracts pass. No separate
Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug SHA256
`f97a7890eab5930433e03801788b6cae73a37986ecfbeec8b421d005d418bc74`.
Source stayed frozen through full and afterward. About 28 GiB free at full start,
18 GiB after; no storage failure or cleanup in this slice.

Documentation closeout: 118 capability records, 107 local links across five
changed Markdown files (baseline 2a9814e), unchanged capability implementation/
maturity and historical evidence, and whitespace PASS. Logs:
`/tmp/ios-a02-xpc-native-docs-catalog.log` and
`/tmp/ios-a02-xpc-native-docs-links.log`. Source and full gate stay unchanged.

A02/class 3/overall goal remain active; class 4 has not started. The read-only
next-service audit records candidate native deadlines, address/integer/framing
adapters in `/tmp/ios-a02-xpc-native-next-services.log`; each still needs role and
behavior review. DProxy textual observations, other codecs/typed guards/created
outcomes, stream/JSON/callback/binding and L01/L03/A03/platform/device conditions
remain. All 31 tasks/subtasks and capability maturity stay. No push/publication,
history rewrite or reference checkout edits.
