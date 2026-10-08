# Successful tunnel scan bookkeeping

Implementation `6bceff76ce31bac1006b4e97f1d0d00a28731abc`, parent
`b6af4fd9dd775bc0d49a1702a3a6cc6c2cd7245f`. Accepted on 2026-10-08,
Linux x86_64. This delivers the L01/L02 successful-candidate bookkeeping work
item; the main tasks retain their other acceptance requirements.

A published successful candidate no longer queues a failure-table writer while
later candidates still need backoff reads. With an existing failure row,
`try_write` clears the exact observed row immediately when uncontended. A
contested cleanup becomes an owned success intent, releases the connection
permit and device serial guard, and joins failure intents after the connection
window. A fresh success with no observed failure row needs no writer.

Deferred success and failure intents share the existing checked one-second
bookkeeping budget and whole-scan deadline. The fast path uses the existing
candidate and whole-scan clocks. A cleanup clock crossing restores the exact
unobserved row, then defers success; it cannot turn a published connection into
a failed connection or increment backoff. A timely removal prefix remains
committed if a later removal expires. Bookkeeping timeout prevents new readiness
and keeps unwritten old rows. Whole expiry and global cancellation retain the
previous scan-control rules.

Each actual failure write creates a private row version. Success cleanup checks
that version and its device/global ticket, so a late success cannot delete a
newer failure even when its identity, timestamp, count and permit coincide.
The version is internal; public options, wire formats and error enums are
unchanged. Deferred cleanup changes contention timing and needs the existing
pre-1.0 compatibility review.

Eleven added regressions cover the fifth candidate entering while four
successful publications face a held failure-table reader, bounded flush,
release-and-clear, whole expiry/clear/close, later failure and epoch protection,
before/after-removal rollback, one shared total, stop-A/continue-B, immediate
slot return at full failure capacity and the candidate-clock fast-path crossing.
One fixture uses actual Core userspace workers and the original native Arcs,
joins, duplex and loopback EOF, registry/permit drain and recovered task count.
It supplies private synthetic discovery metadata and performs resource recovery
before checking the changed behavior. No device or OS TUN was used.

Validation ran serially in the main workspace after integration and formatting:

- `CARGO_INCREMENTAL=0 cargo test -p ios-cli --all-features --locked cmd::tunnel::`:
  268 passed, zero failed, one existing ignored test.
- `CARGO_INCREMENTAL=0 python3 scripts/validate.py full --python .venv/bin/python`:
  exit zero; Rust 2792 and Python host 13 passed, zero failed, two existing ignored
  tests. CLI 1131, Core 1496 and public contracts 55 passed. Format, workspace
  all-target/all-feature check and strict Clippy, MSRV, 66 individual feature
  checks/four groups, doctests/examples, linked C ABI, rebuilt wheel/sdist
  installation, 73 tooling tests, capability inventory, raw diagnostics and host
  workflow/CLI/backup contracts passed.

Both runs verified 470 product-file hashes unchanged during validation. Logs,
command/exit records, source manifests and the accepted binary/patch hashes are
in `target/validation/tunnel-success-bookkeeping-2026-10-08/`.
Patch SHA256: `f1d6f70bb9deecd54a8d654175c954c75265a78eabaf57f8e64a71687d3d9dad`.
CLI SHA256: `566fa0021bab6bb981c1bd23630e533e1d87ec28947412dda1a3e97329918757`.

Twenty-two real-device checks remain NOT_RUN. Other platforms, arbitrary retained
payload bytes, HTTP/upgrades/blocking startup, physical freshness/reuse/restart,
reconnect and resource/performance baselines retain their original requirements.
Q04 and all its children remain excluded. No main-task completion or capability
maturity upgrade follows from this host work item.
