# Guarded native sources: host validation

Date: 2026-10-03. A02/class 3 source `37b09e0bd822bce725e74a01a78eedaeceb4688e`, branch
`feat/device-service-updates-20260927`, baseline
`e6e274ea175e327f00b8abb337bda69c7491b430`. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, workspace Python 3.12.13.
This accepts source ownership and existing host rejection behavior.

Eight received-data adapters previously discarded native objects into Protocol
strings. They now use existing Io variants with private fixed context,
InvalidData and an optional full-width declared u64. Their class remains
Protocol; this IO container records a received-data role, not an OS failure.
Native sources remain available without formatting or message matching.
Debug shows fixed context and declared numbers, excluding native payload text.

| Adapter | Original native object | Production reachability |
| --- | --- | --- |
| AFC header | zerocopy CastError | Fixed 40-byte source, header size 40/alignment 1: guarded on every host |
| AFC entire_len | TryFromIntError | Can fail when usize is narrower than u64 |
| AFC this_len | TryFromIntError | Guarded on every host: this <= entire, whose usize conversion already succeeded |
| AFC status/file handle | Two TryFromSliceError | Exact eight-byte slices after existing length checks |
| DTX header | TryFromSliceError | Exact 32-byte slice after existing length check |
| FileService length | TryFromSliceError | Exact four-byte slice from a fixed header |
| Remote symbols count | TryFromIntError | Can fail when usize is narrower than u64 |

Six adapters are unreachable under the current guards on any supported host.
Only entire_len and symbol count need narrower-host runtime evidence.
On this 64-bit host none of the eight production native-failure branches is
claimed exercised. The u64::MAX production tests reach existing frame/count
ceilings; conditional narrower-host assertions compile but were not run.

AFC uses zerocopy's map_src to own the original CastError bytes as Box<[u8]>.
Only the failure path copies the production caller's fixed 40-byte input;
successful borrowed decoding stays. A private 39-byte source fixture proves
that the exact adapter outlives the stack source, retains Size kind/original
bytes and safe Debug. It is not a short-header production recv failure.
The generic helper uses actual u32 overflow and a seven-byte array conversion
to prove native integer/slice sources and Protocol role. It does not substitute
those fixtures for impossible production paths.

AFC's private recv test rejects u64::MAX after exactly 40 input bytes, no body
allocation or writes. The public AfcClient test runs joined owned duplex peers,
calls list_dir("/"), checks the exact 42-byte request and local EOF after
client destruction, under a one-second join bound. Its exhaustive error match
compiles through public paths. Both receive the original frame-limit Protocol
on this host; local EOF does not certify remote cleanup/reuse.

Seven source files, 231 insertions/27 deletions, including four internal and one
external tests. Entire production bodies match baseline after normalizing only
the eight original error callbacks and removing the additive private helper.
Read/order/magic/length/allocation/status/limits, successful wire and original
tests stay. Existing context policy, root exports, public exhaustive error
families/fields/signatures, core feature list, classifier, AFC/DTX layouts and
H2/XPC codecs are unchanged. Returned variant/Display/immediate-source and
diagnostic correlation can change if an affected branch occurs. Migrate text
or Protocol-variant matching to typed class and bounded native source traversal;
explicit sources follow the [error privacy contract](../error-contract.md).
No new retry, deadline, cleanup, rollback, transfer or reuse guarantee.

The existing zerocopy 0.8 std feature supplies its native StdError on MSRV.
Temporarily omitting std and running the exact minimum afc check on 1.80
failed with E0277 (CastError lacks StdError), terminal exit 101. Restoring std
passed the same check, terminal exit 0. This is a dependency/MSRV compile
condition, not a behavioral negative control. No dependency version, lockfile
or public core-feature change. Final source includes std throughout full.

Logs share `/tmp/ios-a02-guarded-sources-`. Original snapshots, source-diff.patch
and code-audit-final.log retain the production comparison. Earlier audit
formatter-comma mismatch was corrected in the audit only, without source edits.

| Check | Actual result / log suffix |
| --- | --- |
| New internal all-feature/minimum combination | 4/4 PASS / after-final.log, minimal-new.log |
| AFC/DTX/FileService/symbols | 20/46/18/11 PASS / afc.log, dtx.log, fileservice.log, symbols.log |
| Public all-feature/minimum combination | 37/17 PASS / public.log, minimal-public.log |
| Minimum MSRV afc restored / without std | exit 0 / exit 101 E0277 / msrv-final.log, msrv-without-std.log |
| Strict core Clippy | exit 0 / clippy.log |
| First canonical full | terminal exit 0 / full.log |

Minimum combination: afc,dtx,fileservice,fetchsymbols, no default features.
Full: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`.
2073 Rust/13 Python PASS, zero FAIL, two existing ignored, fourteen Rust
result groups and one Python group. Fmt/check/strict workspace Clippy/MSRV,
66 feature builds/four groups, 7+7 doctests/three snippets, C ABI/malformed/
checked-list, 3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help
records, 38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI and
three backup contracts pass. No separate Chromium or installed 3.9/3.13 rerun.
Source stayed frozen during full and documentation; all handles terminated.

Source diff SHA256:
`859736faeb608d2c6aa836d05ba58515c50488348d8f30a1194e1f16d5e1a277`.
Tested ios 0.1.13 debug SHA256: `a3820d7a3c9009b4bfe32ea910316c81ec8a54b2bdaf5519041097fd6183fbba`.
Disk: about 55 GiB after narrow checks, 34 GiB at finish; no cleanup or
storage failure. Eight documents are committed separately; existing 118
capability records/maturity/history stay with one new host entry.
Seven Markdown files/175 local links, whitespace and source/binary
hashes pass doc-audit.log.

A02/class 3/overall goal stay active; class 4 has not started; all 31 tasks and
subtasks remain. Next: mDNS/CDTunnel and remaining connection native sources,
then typed decisions/stream/JSON/callback/binding work. L01/L03/A03/platform/
device conditions remain. No physical device/daemon/credentials/Apple operation,
reference edit, push, publication, history rewrite or maturity upgrade.
See the [public contract](../public-contract.md) and
[preceding recorder gate](dproxy-recording-observations-2026-10-03.md).
