# Explicit existing-trust SDK device query acceptance

Linux x86_64 host boundary, 2026-10-09. Implementation `6f582c9a6eddfc49f7e161f44ad20f507c7986d2`:
13 source/test files, additive finite queries and caller-loaded pair-record parsing.
[API, budgets and ownership](../owned-device-query.md).

21 new owned native fixtures exercise real TCP/Unix usbmux frames, Lockdown
requests and actual Rustls TLS. They cover typed fields/root projection and
missing/empty states; whole metadata selection/admission; exact/+1 wire/UTF-8
limits and invalid frame headers; remote Error precedence; actual IO/parser/TLS
causes; partial reply cancellation, totals across all stages/unpolled wait, fresh
DeviceID discovery, last-client drop, no-queue/two-owner isolation and close with
outstanding unpolled jobs. Two pair-byte tests cover exact 1MiB, +1, empty and
original parser cause/safe diagnostics. Existing owner/checked-session contracts
remain in the final full run. No physical device is represented by these peers.

The first compile found an ambiguous dictionary key conversion and unused import;
root supplied String keys and removed the unused import. The first Core run then
passed1609/failed2: the old partial-reply cancellation raced consumption (peer RST),
and a tiny TCP receive window delayed draining the blocked-send fixture beyond its
cleanup bound. Root added a test-only observation of actual reply-prefix charge,
waited for consumption of the complete 4-byte prefix before cancel, and used the
actual Unix endpoint for the 8MiB blocked StartSession send. Strict original EOF,
original task joins, incomplete-send and permit/count assertions remain required.
The corrected dedicated 21 tests pass in3.02s (53.999s including compilation).

Minimal no-default-feature Core:335PASS/0FAIL,17.743s. These affected/minimal
receipts precede the test-asset sharing correction below; fixture semantics and
public behavior stay the same. Final feature checks compile all affected targets,
and the final full Rust run actually executes the queries and old contracts.

Initial full entry passed fmt and workspace all-target/all-feature check (31.1s),
then strict Clippy rejected loading one TLS fixture as two modules. Root made it
a single test-only shared module; an initial resume caught its missing crate-local
constant visibility. Both failed commands/logs are retained. The final resume runs
fmt, strict lint and every remaining required full stage serially/fail-fast:
exit0 in863.13s. Only four test/module-visibility
files differ from the prior all-target check manifest. That check remains separate
evidence; no single full-entry exit0 is claimed.

Final primary Rust2977PASS/0FAIL/Python13PASS. Five ordinary ignored tests comprise
two existing and three actual Node clients; their prior explicit final execution
is in [CDP acceptance](cdp-client-workflow-2026-10-09.md), not inferred from ignored
counts. MSRV1.80.1,66 feature checks/four groups, default/full doctests7each,
three downstream examples, linked C ABI, rebuilt/installed wheel and sdist,
73 tooling tests,118 capabilities,48 raw captures, five host workflows,
95 CLI and three backup contracts pass. Device rows remain22NOT_RUN.
All505 live frozen paths and all505 committed product blobs at the implementation
commit match the final accepted manifest. Root alone executed Rust jobs; no
independent agent compiled. No build/validation process remains for this batch.

Evidence directory: target/validation/sdk-device-query-2026-10-09/; failed and
successful logs/results/freeze manifests, resume-full.py, complete
frozen-implementation.patch including new files and accepted-evidence.json.

Final resume log SHA256: `a32d56b429d7be0915957f51bfb44ae2e5bd984e4eeb6f0b2ee9bd6a83d19801`.
Patch SHA256: `c37a90dee2fcd78aea10466619c7f44c5da4571cd3048978fae79eb7b3c270ec`.
Resume script SHA256: `e2ccb51885f8f24242ef241fab82fb3a9082e59d2591d7138a93c35abfa2ced7`.
CLI SHA256: `fc1e4debe00da8932f4278856fd253c48988178c9a7b2a990d0a7ba80578c546`.

A03 is still in progress: Python/C device query integration, applications/files/
logs, binding lifetime/error/cancellation examples and their actual acceptance
remain. Device tests are deferred to the user's later arrangement; missing
platform/physical evidence is preserved, maturity is unchanged and Q04/all children
remain excluded. This delivery is the finite Rust device-query subitem, not a
completion claim based on aggregate test counts.
