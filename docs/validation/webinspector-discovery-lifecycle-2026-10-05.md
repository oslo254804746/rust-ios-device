# L01 WebInspector CLI discovery primary errors

Host acceptance,2026-10-05. Tested implementation:
`db7960ae4d8643c1c8849e4a8795a21cdc821ee6`, based on
`e1befa3e25fa0b0c2b4332c8e69698dbde2cbfc9`. Linux x86_64,
Rust1.97.1/MSRV1.80.1/project Python3.12.13. Continues the
[cache profile](webinspector-cache-lifecycle-2026-10-05.md), preserving the
earlier client/session owners. L01/class4/all31 remain active.

Both CLI wait_for_application and wait_for_inspectable_page formerly consumed
every issued Timeout as ordinary idle. When partial input or an automatic
listing write invalidated the client, the next application-list request replaced
the primary native timeout with static invalid-client Protocol. They now retry
only while client.is_usable, otherwise return the original error immediately.
Native Elapsed/duration and Timeout classification survive; a late complete
reply cannot be consumed by another discovery request. This error behavior
migration needs pre-1.0 release review.

Empty usable idle still retries. Successful application/page/cached selection,
explicit page-key precedence over bundle filtering, existing absolute-deadline
helpers/500ms idle cap, CLI grammar/output and core signatures/models/wire/error
families stay. Apart from two guard expressions and the new child test module,
the entire original workflow file is byte-identical. Exactly two source/test
files,384insertions/2deletions; no original assertions/core/dependency/reference
changes, reader task/remote cleanup/ACK/retry policy or new public API.

| Seven actual owned-stream fixtures, each both workflows | Observation |
| --- | --- |
| Partial header |Original issued20ms/native Elapsed, invalid client, queued complete late frame and no further application-list writes |
| Partial body |Same ownership/source result after the full header and eight body bytes |
| Automatic listing |Real connected snapshot triggers a send into a64-byte duplex peer that does not read; partial outgoing listing/native timeout preserved |
| Empty usable idle |Observe a second actual list request, then actual application/page selection succeeds; a subsequent cached match sends nothing; explicit page key wins over unrelated bundle |
| Partial EOF |Original native IO UnexpectedEof, no synthesized Elapsed; invalid client rejects later guarded work |
| Malformed plist |Original native plist::Error and Plist family; no discovery retry |
| Resource rejection |Actual1025-entry connected list retains Io/ResourceLimit and invalidates; no idle success/retry |

Fixtures use supplied ServiceStream boxes/owned duplex streams,20ms configured
idle and3s outer bounds; no device guard is forged. No device/browser/OS signal,
baseline-wide old-source negative control, benchmark/RSS/other-platform result.
The remaining frontend setup/whole-flow totals, cached due-deadline admission,
pinned signal/join/output/independent cleanup and interactive/server owners are
not certified by this narrow fix. Existing output macros already propagate IO;
output within the same total/cleanup scope still needs owner evidence.

First84258 exited101 with23 test-only E0283 dictionary-key type-inference
diagnostics; no test execution/PASS. Explicit String keys fixed the fixtures.
Corrected60870 terminal0:7 PASS. Sequential42910 terminal0: minimum CLI7,
CLI WebInspector47, unchanged Core96+public5+integration1=102, core-only
webinspector cache minimum17, minimum CLI all-target check, original stack5,
strict Core/CLI Clippy and fmt PASS. CLI's dependency still enables core full;
the core-only webinspector check is separate. All existing builds were terminal
before edits or another pipeline; observation timeouts were not exits.

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full92892 terminal0: **2364 Rust/13 Python**, CLI759/Core1443/public52,
zero FAIL/two existing ignored. Every section11 split gate PASS: workspace
fmt/check/strict Clippy/MSRV;66 feature checks/four runtime groups;7+7 doctests/
three snippets; C ABI;3.12 wheel/sdist build/install/import;59 tooling/118 actual
help/38 raw/five host/55 actual CLI/three backup contracts. Chromium/installed
3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

Focused CLI dev cache clean1301files/5.1GiB initially, free695MiB before/4.5GiB
after. After all gates terminal and2.8GiB free, focused Core dev cache clean
removed65236files/72.1GiB, leaving71GiB; full rebuilt the relevant targets and
ended about49GiB free. No source/reference deletion or storage failure.
Frozen two-file source SHA256:
`810c27f44d0b5f192dd77904694bc4f16926ab00b2c11c74d21db3ad1bbd19d2`.
Final ios debug binary SHA256:
`2455e4279eb5d7313278e12c9ff827487f5d5ee9330e347dd7bdd7e255d53a85`.
Actual commit equals the frozen tested patch. Local evidence/audits prefix:
/tmp/ios-l01-webinspector-discovery-. Reference HEAD/status unchanged.

All31 tasks/subtasks/history and capability118 bodies/maturity/prior evidence
stay, with one host entry. Continue WebInspector frontend/server owners,
other L01/raw borrowed/multi-request/long-stream/manual-pairing/blocking owners,
L02–L04/Q04 decoded plist/fuzz/resource evidence and classes5–9. No subagent/
push/publication/history rewrite/reference edit. Host does not replace device,
platform, credentials or protocol conditions or complete the overall goal.

Documentation audit PASS: eight documents/seven Markdown/348 local links and
source/actual-help inventory118. All31 task/subtask/history prefix and118
capability bodies/maturity/prior evidence remain; only the L01 overview/final
delivery and one host entry added. Exact source/binary/full/narrow/compile/gate/
cache-clean provenance matches; source stayed frozen during docs and reference
HEAD/status remain unchanged.
