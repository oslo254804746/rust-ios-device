# Configurable manager reconnect episodes

Accepted on 2026-10-08, Linux x86_64. Implementation: `1a604afea90edbc54b98a21f81fc7f5e29effb04`.
This completes the configured automatic-manager host work item, not L03 as a whole.

## Delivered behavior

`tunnel serve` adds `--retry-initial-delay-ms` (30000 by default),
`--retry-max-delay-ms` (300000), `--retry-max-attempts` and `--retry-total-ms`.
Nonzero/cap ordering and checked clock validation occurs before manager startup.
Default scans retain their existing 30/60/120/240/300-second failure-table
schedule with unlimited retries. Explicit values equal to the defaults retain
that policy. HTTP JSON and the default warning-level output remain unchanged.

A changed policy retains one authoritative episode per admitted identity and
lifecycle incarnation. Its total begins only when a Source attempt is admitted,
then includes attempts, idle backoff and later queue/metadata waits. No permit or
native-owner-capacity admission creates an attempt or restarts the clock.
Attempt/failure, total and capped exponential backoff survive an abandoned or
contested failure-table writer. Exhaustion prevents another Source call for that
identity. Identity change, confirmed detach, authorized fresh success, clear and
shutdown have explicit version/ticket fences. An obsolete success cannot clear
another attempt's newer episode. Exhausted rows deliberately retain their
existing failure reservation until recovery/reconciliation/clear; the shared
4096 limit provides backpressure. It is a logical record-count bound, not a
complete payload-byte/RSS/CPU certification.

Verbose INFO state events contain hashed device IDs, phase/reason and typed
error class. `schedule_source=episode` additionally reports authoritative
counters, clock and next delay. Default `legacy_failure_table` events omit those
cumulative fields: the temporary attempt row does not own the retained schedule.
A pending admitted attempt dropped after its current total expires records one
local budget Timeout; this state does not invent an acquired native Elapsed.
Unexpired caller Drop remains Interrupted, obsolete ticket/version adds no new
failure, and a native error acquired across the clock keeps its original class.
Private CLI resource decisions use the same classifier as HTTP/machine errors.

The ledger owns no background task or event queue and retains at most one
pending event per row. Emission occurs outside synchronous ledger Mutex and
watch borrows. Existing per-device Tokio serial and scan mutex lifetimes remain;
this is not a claim that emission occurs outside every lock. Original native
owners remain in the bounded retirement registry and retain their actual joins.
Production capacity prevents uncontrolled accumulation, but the implementation
does not guarantee every retired worker has joined before every later retry.

## Validation

Twenty-six additive manager regressions
cover actual scans, paused-clock schedules, subscriber fields, quotas, identity,
epoch/version, idle/queue/metadata totals, Source-error priority and native owners.
An automatic native A remains Pending until its actual 1.5-second total ends;
while it is pending, actual HTTP GET/DELETE for native B each finish within a
one-second budget. Tests verify original owner joins, task/registry/permit
recovery and protocol peer release before changed behavior assertions.

Affected CLI all-feature tests passed: 1185, zero failed, one existing ignored.
The final `CARGO_INCREMENTAL=0 python3 scripts/validate.py full --python
.venv/bin/python` exited zero in 277.667 seconds: Rust 2888 and Python host 13
passed, zero failed, two existing ignored. Format, workspace all-target check,
strict Clippy, MSRV 1.80.1, 66 individual feature checks/four groups, docs/examples,
linked C ABI, rebuilt wheel/sdist, installed contracts, 73 tooling tests, 118-record
inventory, 43 raw diagnostic cases, five host workflows, 95 CLI contracts and
three backup contracts passed. All 481 live product paths and hashes were checked
unchanged before commit. Agents used independent worktrees and did not compile.

Evidence: `target/validation/manager-reconnect-sdk-primitives-2026-10-08/`, final
`full-manager-reviewed-result.json` and `accepted-reviewed-evidence.json`. The
previous `full-final` pass predates manager review corrections and is the Core
prerequisite acceptance record, not this manager's final evidence. A fixture used
a reqwest JSON API that the existing feature profile does not provide; it now
parses the actual response text through serde_json without adding dependencies
or weakening assertions. Failed records remain archived.

Full log SHA256: `9f473dc9d6d70e18e5651b92bff01229a7140215a23ec78bb78124e26f8c2a3f`.
Reviewed frozen implementation patch SHA256:
`377c5705bbfb25b6b04b6fa1dff3039bef63083f3275cd6986d2a36ef09cadf3`.
CLI SHA256: `3b3bccd4bf6fbb955b2752a4f40a182a5d7a9529ae64c86029cbba4f95d481e6`.

## Remaining acceptance and migration

Choose serve retry policy/budgets explicitly when finite automatic episodes are
needed. Exhaustion persists until a qualifying recovery/reset; polling cannot
silently renew it. A stop cancels the old incarnation, but a later scan may still
rediscover that device. Durable cross-scan stop and manual/directed Wi-Fi recovery
intent are the next L03 work item, not acceptance granted by this batch.

Pasteboard Darwin-watch baseline/nonce/changeCount and disconnected-gap semantics
remain pending. No automatic pairing, installation or erase replay was added.
Physical unplug/Wi-Fi/service restart/address-change workflows, 22 device rows,
macOS/Windows, remoted coexistence and L04 performance/long-run evidence remain
open. L03 and the other main tasks retain every original requirement; capability
maturity is unchanged. Q04 and all its children remain excluded.
