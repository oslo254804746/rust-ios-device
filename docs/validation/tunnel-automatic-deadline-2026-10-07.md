# Automatic tunnel candidate deadline boundaries

Implementation `c92503ded8a60515a272f4afb1dba99085b130dc`, parent `034d932148e329ed1564023526b9ebe21b948d0f`; Linux x86_64,
2026-10-07, Rust1.97.1/MSRV1.80.1, project Python3.12.13.
The effective goal remains all30 original main tasks and every child excluding
Q04:9complete/21unfinished (9notstarted/12inprogress-or-conditions).

## Candidate behavior and retained ownership

Automatic scan candidates previously used a relative Tokio timeout around their
serial/cache/backoff/permit/connect/publication body. Tokio can accept successful
work that crosses its deadline within one poll, and ready work could run even
under a zero budget. An unrepresentable duration used the unbounded timer path
and contacted Source instead of rejecting that internal configuration.

Each candidate now constructs one checked absolute deadline before its serial
wait. The established15s default and independent candidate admission/window
policy remain. The deadline covers serial/cache/backoff/permit/connection and is
passed into the same native-handoff/metadata/query/publication fences used by
manual starts. Late Source success cannot read metadata; late metadata success
cannot query USB or publish. A real table insertion crossing the clock removes
its unobserved row while table and lifecycle epoch fences stay held. After both
release, the late lease retires and stops its exact original native Arc; the
existing bounded registry retains its original joins for explicit shutdown.

The independently polled timer still produces actual Tokio Elapsed with the
previous scanned timeout context. The shared wrapper preserves already acquired
original IO even if its poll crossed the clock. Zero never polls Source or creates
an owner/failure record. Unrepresentable internal budgets reject as InvalidInput
before Source, without a panic. Existing native errors, lifecycle cancellation,
failure identity/backoff/slot semantics, successful output and wire formats stay.
The manual constructor keeps its previous context without another async wrapper.

This is a per-candidate total. Queued candidates start their clock when admitted
into the existing four-candidate window. Post-attempt failure bookkeeping remains
outside that clock and can await its lock. Whole-scan post-paired handoff,
reconciliation/readiness/failure-record totals, metadata logical byte limits,
generic HTTP/upgrades/blocking workers, startup signals and physical freshness,
ID reuse, daemon restart/manual epochs remain. A timer cannot preempt synchronous
Source internals, metadata, CPU or Drop within a poll. Public APIs, error enums,
options, dependencies, features, Cargo.lock, Core and bindings remain; late-result
and zero/overflow timing corrections require pre-1.0 compatibility review.

## Host evidence and wrong-old baseline

4 source files214+/28-;7 additive tests. Three actual automatic candidate workflow cases
create real userspace Tokio workers over DuplexStream and private discovery/device
metadata. Two automatic zero/overflow cases intentionally create no native runtime
after the fix. Two exercise the actual scanned deadline wrapper for native
Elapsed/InvalidInput and retained BrokenPipe/Disconnected. The controlled200ms
clock/250ms one-poll synchronous delay establishes causal fences, not performance,
CPU preemption, RSS or long-run acceptance.

Workflow cases cover late Source success before metadata, late metadata before
USB querying, and actual insertion rollback. They require one failed-attempt
record and an independently retired original native owner. Every actual workflow
joins the exact original Arc, observes real stream EOF and task-count recovery
before regression assertions. Zero/overflow cases check no Source call, metadata,
query, publication, failure entry or occupied native slot, and no panic.

The first five automatic tests ran against accepted034d932 production without
any production edit: session89814 terminal101,0PASS/5FAIL. All five reached actual
native recovery before their late-row/admission regression assertions. Exact
test-only baseline sources, every452 product hash and the failure log are retained.
After the fix and two wrapper/category cases,28534 terminal0/all7 PASS.
The prior manual post-insert mutation control remains separate historical
evidence; it is not an automatic replacement test or a new mutation run here.
Read-only review found no required production change. Actual automatic late
SourceFailure is not separately exercised: IO precedence evidence here is the
scanned wrapper plus source wiring review. The literal scanned context is
statically reviewed, not asserted by a new test. Failure-slot reservation and
successful failed-record removal remain inside the clock; failed-attempt record
writing remains outside it.

## Accepted serial validation

Runner52757 terminal0, all seven frozen serial gates. Root alone runs Rust and
makes no product/source/doc edits while any validation process lives. Read-only
review outcome is recorded alongside the source evidence; no agent builds,
tests or changes product source.452 product hashes are checked before and after
each gate. Ignored exact source snapshots, command/exit/log hashes, the baseline
failure, source review and frozen patch reside in
`target/validation/automatic-deadline-crossing-2026-10-07/`.

| Check | Result |
| --- | --- |
| CLI tunnel regression |208PASS/1existing ignored, including all7 new |
| Minimum CLI all-targets/original stack tests |PASS/5PASS |
| Strict workspace Clippy/format |PASS/PASS |
| Section11 full Rust/Python split |2732Rust/13Python,0FAIL/2existing ignored |
| Project .venv tooling |73PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full CLI1071/Core1496/public55. Workspace check/fmt/strict Clippy,MSRV1.80.1,
66features/four groups,7+7doctests/three downstream snippets,C ABI,Python3.12
rebuilt wheel/sdist/install/import,tooling73/help118/raw38/host5/CLI95/backup3 PASS.
22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. No maturity,physical,
OS TUN, remote ACK/rollback, resource/performance/long-run or platform certification.

Frozen patch SHA256 `dd242f5a526c23ec2a384552370be54e6e90230f6afd1b1ebe3ecb4e093549ab`.
Actual debug ios SHA256 `4c9f6ba765aa9d5b69a9a252513d70b271e95d5d9e8eaca587c7a095a21fdbca`.
Implementation equals tested freeze; original reference HEADs/statuses remain.
CARGO_INCREMENTAL=0 throughout, no cache deletion/push/publication/history rewrite,
credential/device/Apple/elevated TUN action. Q04 and all its children remain
excluded; UID risk stays untouched. Every other original acceptance condition
remains required. This profile closes no main task, class4 or overall goal.
