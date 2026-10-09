# Manager route intent and durable stop host acceptance

Accepted on 2026-10-09, Linux x86_64. Implementation:
`8947fc1458d8d0ccb9fc8a67d635b23983cadf19` (17 files).
This completes the manual route/durable stop **host subitem**, not L03.
[Behavior and migration](../manager-route-intent.md).

## Delivered boundary

Successful actual POST routes retain their family/filter and original grant
revision/expiry. Recovery invokes fresh Source discovery; a live shortcut keeps
the route that actually ran. DELETE, including 404, persists stopped intent across
scans/replug. Failed/denied/cancelled starts do not reactivate it. A reusable clear
or successful authorized explicit start can reset it. Durable states share the
existing 4096 coordinator ceiling, with no eviction to admit unknown keys.

Publication checks authority/connection clocks after all mutable intent locks and
after candidate mutation. Expiry restores exact Snapshot/pending identities,
watch state and any displaced original native row before readers can observe it.
Late native owners retire outside table/watch guards. Successful intent versions
fence obsolete failure cleanup. Known manual candidates can recover even when an
admitted snapshot also contains unknown keys refused by the full gate table.

Fixed USB/paired/coordinator cause slots preserve original error priority and
secondary completion causes through the existing bounded caller-Drop history.
There is no additional device task, retry ledger or failure-permit namespace.
Logical key/count limits do not certify total payload bytes, CPU or RSS.

## Actual validation and corrections

Root alone ran Rust tools, with process admission checks and frozen source for
every validation job. Agents used independent worktrees and ran no Rust tools.
The final 511 live product hashes and all 511 committed blobs were verified.

Four dedicated real-clock HTTP tests passed all twelve state-lock, pending-lock
and candidate crossings (live shortcut/new publication, authorization/connection
expiry). Original native joins, strict TCP EOF and resource recovery precede
behavior assertions. The affected final manager regression run passed **331**, zero
failed, one previously existing ignored, in 68.669 seconds including compilation.

The first full entry failed strict Clippy after a successful all-target check:
root grouped connection parameters and named the fixture thread type. The next
entry passed format, workspace all-target check (21.9 seconds) and strict Clippy,
then exposed six fixture failures. The new desired-state read occurred before
old fixtures' intended USB/paired coordinator wait. Root now gates the actual
Source to hold the original later wait, keeps its old assertions and separately
covers the new initial read's expiry/cancel/late-release boundaries. The old
incarnation/exhaustion fixture now uses reusable clear: durable DELETE purposely
requires reauthorization and has its own real HTTP regressions. A retained fixture
failure-record clone owned a real permit; it is dropped before requiring complete
cleanup, with its immutable version/count retained for assertions. Two subsequent
compiler failures from stale references to that dropped clone are archived; the
final version retains both required comparisons without retaining the permit.
No product acceptance assertion was removed or relaxed.

Only four test-only fixture files changed after the successful current-production
all-target check: `intent_tests.rs`, `reconnect_tests.rs`, `scan_setup_tests.rs`
and `snapshot_admission_tests.rs`. Root resumed all other required full scopes,
including final format and all-target strict Clippy. That run exited zero in
**277.877 seconds**: **3006 Rust / 13 Python host** passed, zero failed and five
ordinary ignored. CLI 1230 and Core 1611 passed. MSRV 1.80.1, 66 feature checks/four
groups, docs/examples, linked C ABI, rebuilt installed wheel/sdist, 73 tooling
tests, 118-record inventory, 48 raw diagnostics, five host workflows, 95 CLI
contracts and three backup contracts passed. This is separate evidence for the
prior check plus final remaining scopes, not one full-entry exit-zero claim.
The three ordinary ignored CDP gates retain their separate actual-client
acceptance at `3e697fd`; this batch does not claim to have rerun Node clients.

## Reproduction and evidence

Run the affected scope explicitly:

```bash
cargo test -p ios-cli --bin ios --all-features --locked cmd::tunnel::
```

The complete standard entry remains:

```bash
CARGO_INCREMENTAL=0 python3 scripts/validate.py full --python .venv/bin/python
```

Evidence directory: `target/validation/manager-route-intent-2026-10-09/`.
Retain `full-context-*` for the successful production check and failed fixtures,
`manager-regressions-version-receipt-*` for affected PASS,
`full-fixtures-resumed-*`, `resume-full.py`, `accepted-source.patch` and
`accepted-evidence.json` for final scopes/source/commit correspondence. Original
agent v2 patch/manifest/static record and every failed run remain archived.

- Final scopes log SHA256: `7853781b1c6cfab25a80187d308e83c93cc2c512059bff431d256e745a5c2077`.
- Affected log SHA256: `0a67f22e16279670f204d7a03cc6d4e9747d6d7f1437e4b196178a3ce2b536ff`.
- Frozen final patch SHA256: `607ee80bc08ebb6b3f4d93c527ae0352a52d5271b7a06270ac5613b21d556c44`.
- Resumer SHA256: `104ff7ebb65edf50c9c2dbd5cffe46a06779cc9f46e85a4b28113c3dd99ac728`.
- CLI SHA256: `4ff1fefd44e57a56559f905b65827894c73747d65ba93acb60b77917e27e72ee`.

## Remaining acceptance

Darwin watch baseline/nonce/count/recovery and disconnected-gap semantics remain
in an independent worktree. Physical unplug, Wi-Fi interruption, service restart,
address changes, macOS/Windows/remoted and the remaining original L01/L02/L03/L04
requirements are not completed. The user will arrange physical testing later;
22 device rows remain NOT_RUN. Capability maturity is unchanged. Q04 and all its
children stay excluded. L04 before/after measurements are a distinct work item;
fixture count or native joins do not substitute for resource-growth attribution.
